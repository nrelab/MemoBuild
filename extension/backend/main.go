package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"net"
	"net/http"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/labstack/echo/v4"
	"github.com/labstack/echo/v4/middleware"
	"github.com/sirupsen/logrus"
)

var logger = logrus.New()

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

// BuildRequest triggers a new build
type BuildRequest struct {
	Dockerfile   string `json:"dockerfile"`
	RemoteExec   string `json:"remote_exec"`
	Reproducible bool   `json:"reproducible"`
}

func main() {
	var socketPath string
	flag.StringVar(&socketPath, "socket", "/run/guest-services/backend.sock", "Unix domain socket to listen on")
	flag.Parse()

	_ = os.RemoveAll(socketPath)

	logger.SetOutput(os.Stdout)

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

	ln, err := listen(socketPath)
	if err != nil {
		logger.Fatal(err)
	}
	router.Listener = ln

	// API Routes
	router.GET("/status", getStatus)
	router.GET("/workers", getWorkers)
	router.POST("/build", triggerBuild)
	router.GET("/cache/stats", getCacheStats)

	logger.Fatal(router.Start(""))
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
		healthy := checkWorkerHealth(url)
		workers = append(workers, WorkerStatus{
			ID:      fmt.Sprintf("worker-%d", i+1),
			URL:     url,
			Healthy: healthy,
		})
	}
	return ctx.JSON(http.StatusOK, workers)
}

func triggerBuild(ctx echo.Context) error {
	var req BuildRequest
	if err := json.NewDecoder(ctx.Request().Body).Decode(&req); err != nil {
		return ctx.JSON(http.StatusBadRequest, map[string]string{"error": err.Error()})
	}

	// Run memobuild in background
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

	cmd := exec.Command("memobuild", args...)
	cmd.Env = env

	output, err := cmd.CombinedOutput()
	if err != nil {
		return ctx.JSON(http.StatusInternalServerError, map[string]string{
			"error":  err.Error(),
			"output": string(output),
		})
	}

	return ctx.JSON(http.StatusOK, map[string]string{
		"status": "success",
		"output": string(output),
	})
}

func getCacheStats(ctx echo.Context) error {
	stats := map[string]interface{}{
		"local_cache_dir": os.Getenv("HOME") + "/.memobuild/cache",
		"remote_url":      os.Getenv("MEMOBUILD_REMOTE_URL"),
		"regions":         os.Getenv("MEMOBUILD_REGIONS"),
	}
	return ctx.JSON(http.StatusOK, stats)
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
