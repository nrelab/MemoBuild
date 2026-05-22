package main

import (
	"bytes"
	"context"
	"crypto/tls"
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"strings"
	"sync"
	"time"

	"github.com/labstack/echo/v4"
	"github.com/labstack/echo/v4/middleware"
	"github.com/opentracing/opentracing-go"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promhttp"
	"github.com/redis/go-redis/v9"
	"github.com/sirupsen/logrus"
	"github.com/sony/gobreaker"
	"go.etcd.io/etcd/client/v3"
	"github.com/uber/jaeger-client-go"
	jaegercfg "github.com/uber/jaeger-client-go/config"
)

var (
	logger = logrus.New()
	lb     *RoundRobinLoadBalancer
	rdb    *redis.Client
	rctx   = context.Background()
	// Heartbeat tracking
	workerHeartbeats = make(map[string]time.Time)
	heartbeatMu      sync.RWMutex
	// Dynamic worker registration
	dynamicWorkers   = make(map[string]WorkerStatus)
	dynamicWorkersMu sync.RWMutex
	// Circuit breaker for worker calls
	workerCB *gobreaker.CircuitBreaker
	// Bulkhead: limit concurrent worker calls
	workerSemaphore = make(chan struct{}, 10) // max 10 concurrent
	// Prometheus metrics
	buildRequestsTotal = prometheus.NewCounterVec(
		prometheus.CounterOpts{
			Name: "memobuild_build_requests_total",
			Help: "Total build requests by status",
		},
		[]string{"status"},
	)
	workerHealthGauge = prometheus.NewGaugeVec(
		prometheus.GaugeOpts{
			Name: "memobuild_worker_healthy",
			Help: "Worker health status (1=healthy, 0=unhealthy)",
		},
		[]string{"worker_id"},
	)
	cacheHitsTotal = prometheus.NewCounterVec(
		prometheus.CounterOpts{
			Name: "memobuild_cache_hits_total",
			Help: "Total cache hits by level",
		},
		[]string{"level"},
	)
	cacheMissesTotal = prometheus.NewCounterVec(
		prometheus.CounterOpts{
			Name: "memobuild_cache_misses_total",
			Help: "Total cache misses by level",
		},
		[]string{"level"},
	)
	// etcd client for distributed consensus
	etcdClient *clientv3.Client
)

func init() {
	prometheus.MustRegister(buildRequestsTotal, workerHealthGauge, cacheHitsTotal, cacheMissesTotal)
}

// BuildStatus represents the current state of a MemoBuild execution
type BuildStatus struct {
	Running     bool      `json:"running"`
	LastBuildAt time.Time `json:"last_build_at"`
	TotalNodes  int       `json:"total_nodes"`
	CacheHits   int       `json:"cache_hits"`
	CacheMisses int       `json:"cache_misses"`
	HitRate     float64   `json:"hit_rate"`
	DurationMs  int64     `json:"duration_ms"`
	RemoteExec  string    `json:"remote_exec_url"`
	Workers     []string  `json:"workers"`
}

// WorkerStatus represents a remote worker node
type WorkerStatus struct {
	ID      string `json:"id"`
	URL     string `json:"url"`
	Healthy bool   `json:"healthy"`
}

// RoundRobinLoadBalancer distributes tasks across healthy workers
type RoundRobinLoadBalancer struct {
	workers []WorkerStatus
	current int
	mu      sync.RWMutex
}

func NewRoundRobinLoadBalancer(workers []WorkerStatus) *RoundRobinLoadBalancer {
	return &RoundRobinLoadBalancer{
		workers: workers,
		current: 0,
	}
}

func (lb *RoundRobinLoadBalancer) NextWorker() (WorkerStatus, bool) {
	lb.mu.Lock()
	defer lb.mu.Unlock()

	if len(lb.workers) == 0 {
		return WorkerStatus{}, false
	}

	start := lb.current
	for {
		worker := lb.workers[lb.current]
		lb.current = (lb.current + 1) % len(lb.workers)
		if worker.Healthy {
			return worker, true
		}
		if lb.current == start {
			return WorkerStatus{}, false
		}
	}
}

func (lb *RoundRobinLoadBalancer) UpdateWorkers(newWorkers []WorkerStatus) {
	lb.mu.Lock()
	defer lb.mu.Unlock()
	lb.workers = newWorkers
	lb.current = 0
}

// BuildRequest triggers a new build
type BuildRequest struct {
	Dockerfile   string `json:"dockerfile"`
	RemoteExec   string `json:"remote_exec"`
	Reproducible bool   `json:"reproducible"`
	Priority     int    `json:"priority"` // Higher = more priority
}

func main() {
	var socketPath string
	flag.StringVar(&socketPath, "socket", "/run/guest-services/backend.sock", "Unix domain socket to listen on")
	flag.Parse()

	_ = os.RemoveAll(socketPath)

	logger.SetOutput(os.Stdout)
	logger.SetFormatter(&logrus.JSONFormatter{})

	// TLS configuration for inter-service communication
	tlsCert := os.Getenv("MEMOBUILD_TLS_CERT")
	tlsKey := os.Getenv("MEMOBUILD_TLS_KEY")
	tlsPort := os.Getenv("MEMOBUILD_TLS_PORT")
	if tlsPort == "" {
		tlsPort = "8443"
	}

	logMiddleware := middleware.LoggerWithConfig(middleware.LoggerConfig{
		Skipper: middleware.DefaultSkipper,
		Format: `{"time":"${time_rfc3339_nano}","id":"${id}",` +
			`"method":"${method}","uri":"${uri}",` +
			`"status":${status},"error":"${error}"` +
			`}` + "\n",
		CustomTimeFormat: "2006-01-02 15:04:05.00000",
		Output:           logger.Writer(),
	})

	logger.Infof("Starting MemoBuild Extension backend on %s\n", socketPath)
	router := echo.New()
	router.HideBanner = true
	router.Use(logMiddleware)
	router.Use(middleware.CORS())

	// Authentication middleware
	apiSecret := os.Getenv("MEMOBUILD_API_SECRET")
	if apiSecret != "" {
		router.Use(func(next echo.HandlerFunc) echo.HandlerFunc {
			return func(c echo.Context) error {
				apiKey := c.Request().Header.Get("X-API-Key")
				if apiKey == "" {
					auth := c.Request().Header.Get("Authorization")
					if strings.HasPrefix(auth, "Bearer ") {
						apiKey = strings.TrimPrefix(auth, "Bearer ")
					}
				}
				if apiKey != apiSecret {
					return c.JSON(http.StatusUnauthorized, map[string]string{"error": "unauthorized"})
				}
				return next(c)
			}
		})
	}

	// Audit logging middleware
	auditLogPath := os.Getenv("AUDIT_LOG")
	if auditLogPath != "" {
		router.Use(func(next echo.HandlerFunc) echo.HandlerFunc {
			return func(c echo.Context) error {
				start := time.Now()
				err := next(c)
				latency := time.Since(start)
				logger.WithFields(logrus.Fields{
					"audit":      true,
					"method":     c.Request().Method,
					"path":       c.Path(),
					"status":     c.Response().Status,
					"latency":    latency.String(),
					"remote_ip":  c.RealIP(),
					"user_agent": c.Request().UserAgent(),
				}).Info("Audit log")
				return err
			}
		})
	}

	// Initialize load balancer with configured workers
	workersEnv := os.Getenv("MEMOBUILD_WORKERS")
	workerURLs := parseWorkers(workersEnv)
	initialWorkers := make([]WorkerStatus, 0, len(workerURLs))
	for i, url := range workerURLs {
		healthy := checkWorkerHealth(url)
		initialWorkers = append(initialWorkers, WorkerStatus{
			ID:      fmt.Sprintf("worker-%d", i+1),
			URL:     url,
			Healthy: healthy,
		})
	}
	lb = NewRoundRobinLoadBalancer(initialWorkers)
	updateAllWorkers() // Sync all workers to load balancer

	// Initialize Redis client for message queuing
	redisURL := os.Getenv("REDIS_URL")
	if redisURL == "" {
		redisURL = "localhost:6379"
	}
	rdb = redis.NewClient(&redis.Options{
		Addr: redisURL,
	})

	// Verify Redis connection
	if _, err := rdb.Ping(rctx).Result(); err != nil {
		logger.Warnf("Redis unavailable: %v. Message queue disabled", err)
	} else {
		logger.Info("Redis connected for task queuing")
	}

	// Initialize circuit breaker for worker calls
	workerCB = gobreaker.NewCircuitBreaker(gobreaker.Settings{
		Name:        "worker-calls",
		MaxRequests: 3,
		Interval:    10 * time.Second,
		Timeout:     30 * time.Second,
		ReadyToTrip: func(counts gobreaker.Counts) bool {
			return counts.ConsecutiveFailures > 3
		},
	})

	// Initialize etcd client for distributed consensus
	etcdURL := os.Getenv("ETCD_URL")
	if etcdURL == "" {
		etcdURL = "localhost:2379"
	}
	var err error
	etcdClient, err = clientv3.New(clientv3.Config{
		Endpoints: []string{etcdURL},
	})
	if err != nil {
		logger.Warnf("etcd unavailable: %v. Distributed consensus disabled", err)
	} else {
		logger.Info("etcd connected for distributed consensus")
	}

	// Initialize Jaeger tracer for distributed tracing
	jaegerConfig := jaegercfg.Configuration{
		ServiceName: "memobuild-backend",
		Sampler: &jaegercfg.SamplerConfig{
			Type:  jaeger.SamplerTypeConst,
			Param: 1,
		},
		Reporter: &jaegercfg.ReporterConfig{
			LogSpans: true,
		},
	}
	tracer, closer, err := jaegerConfig.NewTracer()
	if err != nil {
		logger.Warnf("Jaeger unavailable: %v. Distributed tracing disabled", err)
	} else {
		opentracing.SetGlobalTracer(tracer)
		defer closer.Close()
		logger.Info("Jaeger tracer initialized")
	}

	// API Routes
	router.GET("/status", getStatus)
	router.GET("/workers", getWorkers)
	router.POST("/build", triggerBuild)
	router.GET("/cache/stats", getCacheStats)
	router.POST("/workers/heartbeat", handleHeartbeat)
	router.POST("/workers/register", handleWorkerRegister)
	router.GET("/cache/get", getCacheValue)
	router.POST("/cache/set", setCacheValue)
	router.POST("/cache/invalidate", invalidateCacheEndpoint)
	router.POST("/cache/invalidate-prefix", invalidateCacheByPrefixEndpoint)
	router.GET("/metrics", echo.WrapHandler(promhttp.Handler()))
	router.POST("/build/resume", resumeBuildFromCheckpoint)
	router.GET("/cluster/status", getClusterStatus)

	// Start Unix socket server
	unixListener, err := listen(socketPath)
	if err != nil {
		logger.Fatal(err)
	}
	go func() {
		logger.Infof("Starting MemoBuild Extension backend on %s", socketPath)
		if err := http.Serve(unixListener, router); err != nil {
			logger.Fatal(err)
		}
	}()

	// Start TLS server if configured
	if tlsCert != "" && tlsKey != "" {
		cert, err := tls.LoadX509KeyPair(tlsCert, tlsKey)
		if err != nil {
			logger.Fatalf("Failed to load TLS cert/key: %v", err)
		}
		tlsConfig := &tls.Config{Certificates: []tls.Certificate{cert}}
		tlsListener, err := tls.Listen("tcp", ":"+tlsPort, tlsConfig)
		if err != nil {
			logger.Fatalf("Failed to listen TLS: %v", err)
		}
		go func() {
			logger.Infof("Starting TLS server on port %s", tlsPort)
			if err := http.Serve(tlsListener, router); err != nil {
				logger.Fatal(err)
			}
		}()
	}

	// Block main goroutine
	select {}
}

func listen(path string) (net.Listener, error) {
	return net.Listen("unix", path)
}

func getStatus(ctx echo.Context) error {
	status := BuildStatus{
		Running:     false,
		LastBuildAt: time.Now(),
		RemoteExec:  os.Getenv("MEMOBUILD_REMOTE_EXEC"),
		Workers:     parseWorkers(os.Getenv("MEMOBUILD_WORKERS")),
	}
	return ctx.JSON(http.StatusOK, status)
}

func getWorkers(ctx echo.Context) error {
	workersEnv := os.Getenv("MEMOBUILD_WORKERS")
	urls := parseWorkers(workersEnv)

	workers := make([]WorkerStatus, 0, len(urls))
	for i, url := range urls {
		workerID := fmt.Sprintf("worker-%d", i+1)
		httpHealthy := checkWorkerHealth(url)
		heartbeatHealthy := isWorkerHealthy(workerID)
		healthy := httpHealthy && heartbeatHealthy
		workers = append(workers, WorkerStatus{
			ID:      workerID,
			URL:     url,
			Healthy: healthy,
		})
		// Update Prometheus gauge
		if healthy {
			workerHealthGauge.WithLabelValues(workerID).Set(1)
		} else {
			workerHealthGauge.WithLabelValues(workerID).Set(0)
		}
	}
	// Update load balancer with latest worker status
	lb.UpdateWorkers(workers)
	return ctx.JSON(http.StatusOK, workers)
}

func triggerBuild(ctx echo.Context) error {
	var req BuildRequest
	if err := json.NewDecoder(ctx.Request().Body).Decode(&req); err != nil {
		buildRequestsTotal.WithLabelValues("error").Inc()
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": err.Error()})
	}

	// Try Redis message queue first if available
	if rdb != nil {
		if err := publishBuildToQueue(req); err == nil {
			logger.Info("Build task published to Redis queue")
			buildRequestsTotal.WithLabelValues("queued").Inc()
			return ctx.JSON(http.StatusAccepted, map[string]string{
				"status":  "queued",
				"message": "Build task queued in Redis Streams",
			})
		} else {
			logger.Warnf("Failed to publish to Redis queue: %v. Falling back to direct dispatch", err)
		}
	}

	// Fallback to direct worker dispatch via load balancer
	worker, ok := lb.NextWorker()
	if ok {
		logger.Infof("Dispatching build to worker %s", worker.ID)
		buildRequestsTotal.WithLabelValues("dispatched").Inc()
		return forwardBuildToWorker(worker.URL, req, ctx)
	}

	// Final fallback to local execution
	logger.Warn("No healthy workers available, falling back to local build")
	buildRequestsTotal.WithLabelValues("local").Inc()
	return triggerLocalBuild(req, ctx)
}

func forwardBuildToWorker(workerURL string, req BuildRequest, ctx echo.Context) error {
	// Bulkhead: limit concurrent worker calls
	workerSemaphore <- struct{}{}
	defer func() { <-workerSemaphore }()

	jsonData, err := json.Marshal(req)
	if err != nil {
		return ctx.JSON(http.StatusInternalServerError, map[string]string{"error": err.Error()})
	}

	url := workerURL + "/build"
	// Use circuit breaker to protect against worker failures
	body, err := workerCB.Execute(func() (interface{}, error) {
		resp, err := http.Post(url, "application/json", bytes.NewBuffer(jsonData))
		if err != nil {
			return nil, err
		}
		defer resp.Body.Close()
		body, err := io.ReadAll(resp.Body)
		if err != nil {
			return nil, err
		}
		return body, nil
	})

	if err != nil {
		logger.Warnf("Circuit breaker tripped or worker call failed: %v", err)
		return ctx.JSON(http.StatusServiceUnavailable, map[string]string{
			"error": "Worker unavailable (circuit breaker open or call failed): " + err.Error(),
		})
	}

	return ctx.JSON(http.StatusOK, json.RawMessage(body.([]byte)))
}

func handleHeartbeat(ctx echo.Context) error {
	type HeartbeatRequest struct {
		WorkerID string `json:"worker_id"`
	}
	var req HeartbeatRequest
	if err := json.NewDecoder(ctx.Request().Body).Decode(&req); err != nil {
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": err.Error()})
	}
	if req.WorkerID == "" {
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": "worker_id required"})
	}
	heartbeatMu.Lock()
	workerHeartbeats[req.WorkerID] = time.Now()
	heartbeatMu.Unlock()
	logger.Infof("Received heartbeat from worker %s", req.WorkerID)
	return ctx.JSON(http.StatusOK, map[string]string{"status": "ok"})
}

func handleWorkerRegister(ctx echo.Context) error {
	type RegisterRequest struct {
		WorkerID string `json:"worker_id"`
		URL      string `json:"url"`
	}
	var req RegisterRequest
	if err := json.NewDecoder(ctx.Request().Body).Decode(&req); err != nil {
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": err.Error()})
	}
	if req.WorkerID == "" || req.URL == "" {
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": "worker_id and url required"})
	}

	dynamicWorkersMu.Lock()
	dynamicWorkers[req.WorkerID] = WorkerStatus{
		ID:      req.WorkerID,
		URL:     req.URL,
		Healthy: true,
	}
	dynamicWorkersMu.Unlock()

	// Update load balancer with all workers (static + dynamic)
	updateAllWorkers()
	logger.Infof("Registered worker %s at %s", req.WorkerID, req.URL)
	return ctx.JSON(http.StatusOK, map[string]string{"status": "registered"})
}

func updateAllWorkers() {
	// Collect static workers from env
	workersEnv := os.Getenv("MEMOBUILD_WORKERS")
	urls := parseWorkers(workersEnv)
	allWorkers := make([]WorkerStatus, 0)

	for i, url := range urls {
		workerID := fmt.Sprintf("worker-%d", i+1)
		httpHealthy := checkWorkerHealth(url)
		heartbeatHealthy := isWorkerHealthy(workerID)
		allWorkers = append(allWorkers, WorkerStatus{
			ID:      workerID,
			URL:     url,
			Healthy: httpHealthy && heartbeatHealthy,
		})
	}

	// Add dynamic workers
	dynamicWorkersMu.RLock()
	for _, w := range dynamicWorkers {
		heartbeatHealthy := isWorkerHealthy(w.ID)
		allWorkers = append(allWorkers, WorkerStatus{
			ID:      w.ID,
			URL:     w.URL,
			Healthy: heartbeatHealthy,
		})
	}
	dynamicWorkersMu.RUnlock()

	lb.UpdateWorkers(allWorkers)
}

func isWorkerHealthy(workerID string) bool {
	heartbeatMu.RLock()
	defer heartbeatMu.RUnlock()
	lastBeat, ok := workerHeartbeats[workerID]
	if !ok {
		return false
	}
	return time.Since(lastBeat) < 10*time.Second
}

func triggerLocalBuild(req BuildRequest, ctx echo.Context) error {
	args := []string{}
	if req.Dockerfile != "" {
		args = append(args, "--file", req.Dockerfile)
	}
	if req.Reproducible {
		args = append(args, "--reproducible")
	}

	env := os.Environ()
	if req.RemoteExec != "" {
		env = append(env, "MEMOBUILD_REMOTE_EXEC="+req.RemoteExec)
	}

	// Add checkpoint support via environment variable
	checkpointDir := os.Getenv("MEMOBUILD_CHECKPOINTS")
	if checkpointDir == "" {
		checkpointDir = "/tmp/memobuild-checkpoints"
	}
	env = append(env, "MEMOBUILD_CHECKPOINT_DIR="+checkpointDir)

	cmd := exec.Command("memobuild", args...)
	cmd.Env = env

	// Start build with checkpointing
	output, err := cmd.CombinedOutput()
	if err != nil {
		// Save checkpoint on failure for potential restart
		checkpointKey := fmt.Sprintf("build:%s:checkpoint", req.Dockerfile)
		_ = setToCache(checkpointKey, string(output), 1, 1*time.Hour)
		return ctx.JSON(http.StatusInternalServerError, map[string]string{
			"error":       err.Error(),
			"output":      string(output),
			"checkpoint":  checkpointKey,
			"resume_hint": "Use checkpoint key to resume build",
		})
	}

	return ctx.JSON(http.StatusOK, map[string]string{
		"status": "success",
		"output": string(output),
	})
}

func retryWithBackoff(operation func() error, maxRetries int) error {
	var err error
	for i := 0; i <= maxRetries; i++ {
		err = operation()
		if err == nil {
			return nil
		}
		if i < maxRetries {
			backoff := time.Duration(1<<uint(i)) * time.Second // exponential backoff: 1s, 2s, 4s, ...
			logger.Warnf("Operation failed, retrying in %v: %v", backoff, err)
			time.Sleep(backoff)
		}
	}
	return fmt.Errorf("operation failed after %d retries: %w", maxRetries, err)
}

func publishBuildToQueue(req BuildRequest) error {
	if rdb == nil {
		return fmt.Errorf("redis client not initialized")
	}
	jsonData, err := json.Marshal(req)
	if err != nil {
		return err
	}
	// Route to priority-specific stream (higher priority = higher stream)
	stream := fmt.Sprintf("build_tasks:priority:%d", req.Priority)
	return retryWithBackoff(func() error {
		return rdb.XAdd(rctx, &redis.XAddArgs{
			Stream: stream,
			Values: map[string]interface{}{
				"payload":  string(jsonData),
				"priority": req.Priority,
			},
		}).Err()
	}, 3)
}

// Multi-level cache functions
func getFromCache(key string, level int) (string, error) {
	if rdb == nil {
		return "", fmt.Errorf("redis client not initialized")
	}
	redisKey := fmt.Sprintf("cache:l%d:%s", level, key)
	var val string
	err := retryWithBackoff(func() error {
		var err error
		val, err = rdb.Get(rctx, redisKey).Result()
		return err
	}, 2)
	if err == nil {
		cacheHitsTotal.WithLabelValues(fmt.Sprintf("%d", level)).Inc()
	} else if err == redis.Nil {
		cacheMissesTotal.WithLabelValues(fmt.Sprintf("%d", level)).Inc()
	}
	return val, err
}

func setToCache(key string, value string, level int, ttl time.Duration) error {
	if rdb == nil {
		return fmt.Errorf("redis client not initialized")
	}
	redisKey := fmt.Sprintf("cache:l%d:%s", level, key)
	return retryWithBackoff(func() error {
		return rdb.Set(rctx, redisKey, value, ttl).Err()
	}, 2)
}

func invalidateCache(key string, level int) error {
	if rdb == nil {
		return fmt.Errorf("redis client not initialized")
	}
	redisKey := fmt.Sprintf("cache:l%d:%s", level, key)
	return retryWithBackoff(func() error {
		return rdb.Del(rctx, redisKey).Err()
	}, 2)
}

func invalidateCacheByPrefix(prefix string, level int) error {
	if rdb == nil {
		return fmt.Errorf("redis client not initialized")
	}
	redisKey := fmt.Sprintf("cache:l%d:%s*", level, prefix)
	keys, err := rdb.Keys(rctx, redisKey).Result()
	if err != nil {
		return err
	}
	if len(keys) > 0 {
		return rdb.Del(rctx, keys...).Err()
	}
	return nil
}

func getCacheStats(ctx echo.Context) error {
	stats := map[string]interface{}{
		"local_cache_dir": os.Getenv("HOME") + "/.memobuild/cache",
		"remote_url":      os.Getenv("MEMOBUILD_REMOTE_URL"),
		"regions":         os.Getenv("MEMOBUILD_REGIONS"),
		"l1_cache":        "Redis (TTL: 1h)",
		"l2_cache":        "Redis (TTL: 24h)",
		"l3_cache":        "S3 (pending)",
	}
	// Add Redis info if available
	if rdb != nil {
		info, err := rdb.Info(rctx).Result()
		if err == nil {
			stats["redis_info"] = info
		}
	}
	return ctx.JSON(http.StatusOK, stats)
}

func getCacheValue(ctx echo.Context) error {
	key := ctx.QueryParam("key")
	level := ctx.QueryParam("level")
	if key == "" || level == "" {
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": "key and level required"})
	}
	lvl := 1
	fmt.Sscanf(level, "%d", &lvl)
	val, err := getFromCache(key, lvl)
	if err != nil {
		return ctx.JSON(http.StatusNotFound, map[string]string{"error": err.Error()})
	}
	return ctx.JSON(http.StatusOK, map[string]string{"value": val})
}

func setCacheValue(ctx echo.Context) error {
	type SetRequest struct {
		Key   string `json:"key"`
		Value string `json:"value"`
		Level int    `json:"level"`
		TTL   int    `json:"ttl_seconds"`
	}
	var req SetRequest
	if err := json.NewDecoder(ctx.Request().Body).Decode(&req); err != nil {
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": err.Error()})
	}
	ttl := time.Duration(req.TTL) * time.Second
	if ttl == 0 {
		ttl = 24 * time.Hour
	}
	if err := setToCache(req.Key, req.Value, req.Level, ttl); err != nil {
		return ctx.JSON(http.StatusInternalServerError, map[string]string{"error": err.Error()})
	}
	return ctx.JSON(http.StatusOK, map[string]string{"status": "cached"})
}

func invalidateCacheEndpoint(ctx echo.Context) error {
	type InvalidateRequest struct {
		Key   string `json:"key"`
		Level int    `json:"level"`
	}
	var req InvalidateRequest
	if err := json.NewDecoder(ctx.Request().Body).Decode(&req); err != nil {
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": err.Error()})
	}
	if err := invalidateCache(req.Key, req.Level); err != nil {
		return ctx.JSON(http.StatusInternalServerError, map[string]string{"error": err.Error()})
	}
	return ctx.JSON(http.StatusOK, map[string]string{"status": "invalidated"})
}

func invalidateCacheByPrefixEndpoint(ctx echo.Context) error {
	type InvalidatePrefixRequest struct {
		Prefix string `json:"prefix"`
		Level  int    `json:"level"`
	}
	var req InvalidatePrefixRequest
	if err := json.NewDecoder(ctx.Request().Body).Decode(&req); err != nil {
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": err.Error()})
	}
	if err := invalidateCacheByPrefix(req.Prefix, req.Level); err != nil {
		return ctx.JSON(http.StatusInternalServerError, map[string]string{"error": err.Error()})
	}
	return ctx.JSON(http.StatusOK, map[string]string{"status": "invalidated"})
}

func resumeBuildFromCheckpoint(ctx echo.Context) error {
	type ResumeRequest struct {
		CheckpointKey string `json:"checkpoint_key"`
	}
	var req ResumeRequest
	if err := json.NewDecoder(ctx.Request().Body).Decode(&req); err != nil {
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": err.Error()})
	}
	if req.CheckpointKey == "" {
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": "checkpoint_key required"})
	}
	// Retrieve checkpoint from cache
	val, err := getFromCache(req.CheckpointKey, 1)
	if err != nil {
		return ctx.JSON(http.StatusNotFound, map[string]string{"error": "Checkpoint not found: " + err.Error()})
	}
	return ctx.JSON(http.StatusOK, map[string]string{
		"status":     "resumed",
		"checkpoint": val,
	})
}

func getClusterStatus(ctx echo.Context) error {
	status := map[string]interface{}{
		"etcd_connected": etcdClient != nil,
		"redis_connected": rdb != nil,
	}
	if etcdClient != nil {
		// Query etcd for cluster members
		resp, err := etcdClient.MemberList(rctx)
		if err == nil {
			members := make([]string, 0, len(resp.Members))
			for _, m := range resp.Members {
				members = append(members, m.String())
			}
			status["etcd_members"] = members
		}
	}
	return ctx.JSON(http.StatusOK, status)
}

func parseWorkers(raw string) []string {
	if raw == "" {
		return []string{}
	}
	parts := strings.Split(raw, ",")
	result := make([]string, 0, len(parts))
	for _, p := range parts {
		p = strings.TrimSpace(p)
		if p != "" {
			result = append(result, p)
		}
	}
	return result
}

func checkWorkerHealth(url string) bool {
	client := &http.Client{Timeout: 2 * time.Second}
	resp, err := client.Get(url + "/health")
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	return resp.StatusCode == http.StatusOK
}
