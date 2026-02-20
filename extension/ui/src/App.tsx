import React from 'react';
import { createDockerDesktopClient } from '@docker/extension-api-client';
import {
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  CircularProgress,
  Divider,
  Grid,
  LinearProgress,
  Stack,
  TextField,
  Tooltip,
  Typography,
} from '@mui/material';
import BuildIcon from '@mui/icons-material/Build';
import CachedIcon from '@mui/icons-material/Cached';
import CloudIcon from '@mui/icons-material/Cloud';
import CheckCircleIcon from '@mui/icons-material/CheckCircle';
import ErrorIcon from '@mui/icons-material/Error';
import StorageIcon from '@mui/icons-material/Storage';

const client = createDockerDesktopClient();

interface BuildStatus {
  running: boolean;
  last_build_at: string;
  total_nodes: number;
  cache_hits: number;
  cache_misses: number;
  hit_rate: number;
  duration_ms: number;
  remote_exec_url: string;
  workers: string[];
}

interface WorkerStatus {
  id: string;
  url: string;
  healthy: boolean;
}

interface CacheStats {
  local_cache_dir: string;
  remote_url: string;
  regions: string;
}

export function App() {
  const [status, setStatus] = React.useState<BuildStatus | null>(null);
  const [workers, setWorkers] = React.useState<WorkerStatus[]>([]);
  const [cacheStats, setCacheStats] = React.useState<CacheStats | null>(null);
  const [buildOutput, setBuildOutput] = React.useState<string>('');
  const [dockerfile, setDockerfile] = React.useState<string>('Dockerfile');
  const [remoteExec, setRemoteExec] = React.useState<string>('');
  const [reproducible, setReproducible] = React.useState<boolean>(false);
  const [building, setBuilding] = React.useState<boolean>(false);
  const [loading, setLoading] = React.useState<boolean>(true);

  const fetchStatus = async () => {
    try {
      const result = await client.extension.vm?.service?.get('/status');
      setStatus(result as BuildStatus);
    } catch (e) {
      console.error('Failed to fetch status', e);
    }
  };

  const fetchWorkers = async () => {
    try {
      const result = await client.extension.vm?.service?.get('/workers');
      setWorkers(result as WorkerStatus[]);
    } catch (e) {
      console.error('Failed to fetch workers', e);
    }
  };

  const fetchCacheStats = async () => {
    try {
      const result = await client.extension.vm?.service?.get('/cache/stats');
      setCacheStats(result as CacheStats);
    } catch (e) {
      console.error('Failed to fetch cache stats', e);
    }
  };

  const triggerBuild = async () => {
    setBuilding(true);
    setBuildOutput('');
    try {
      const result = await client.extension.vm?.service?.post('/build', {
        dockerfile,
        remote_exec: remoteExec,
        reproducible,
      });
      const r = result as { output?: string; error?: string };
      setBuildOutput(r.output ?? r.error ?? 'Build complete');
    } catch (e: any) {
      setBuildOutput(`Error: ${e.message}`);
    } finally {
      setBuilding(false);
      fetchStatus();
    }
  };

  React.useEffect(() => {
    Promise.all([fetchStatus(), fetchWorkers(), fetchCacheStats()]).finally(() =>
      setLoading(false)
    );
    const interval = setInterval(() => {
      fetchStatus();
      fetchWorkers();
    }, 5000);
    return () => clearInterval(interval);
  }, []);

  const hitRate = status ? Math.round(status.hit_rate * 100) : 0;

  return (
    <Box sx={{ p: 3, maxWidth: 1100, mx: 'auto' }}>
      {/* Header */}
      <Stack direction="row" alignItems="center" spacing={2} sx={{ mb: 3 }}>
        <BuildIcon sx={{ fontSize: 36, color: 'primary.main' }} />
        <Box>
          <Typography variant="h4" fontWeight={700}>
            MemoBuild
          </Typography>
          <Typography variant="body2" color="text.secondary">
            Incremental Build Farm · Smart Caching · Distributed Execution
          </Typography>
        </Box>
      </Stack>

      {loading ? (
        <Box sx={{ display: 'flex', justifyContent: 'center', mt: 8 }}>
          <CircularProgress />
        </Box>
      ) : (
        <Grid container spacing={3}>
          {/* Cache Stats Card */}
          <Grid item xs={12} md={4}>
            <Card variant="outlined" sx={{ height: '100%' }}>
              <CardContent>
                <Stack direction="row" alignItems="center" spacing={1} sx={{ mb: 2 }}>
                  <CachedIcon color="primary" />
                  <Typography variant="h6">Cache Performance</Typography>
                </Stack>
                <Typography variant="h2" fontWeight={700} color="primary.main">
                  {hitRate}%
                </Typography>
                <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
                  Hit Rate
                </Typography>
                <LinearProgress
                  variant="determinate"
                  value={hitRate}
                  sx={{ mb: 2, height: 8, borderRadius: 4 }}
                />
                <Stack direction="row" justifyContent="space-between">
                  <Box>
                    <Typography variant="caption" color="text.secondary">Hits</Typography>
                    <Typography variant="body1" fontWeight={600} color="success.main">
                      {status?.cache_hits ?? 0}
                    </Typography>
                  </Box>
                  <Box>
                    <Typography variant="caption" color="text.secondary">Misses</Typography>
                    <Typography variant="body1" fontWeight={600} color="warning.main">
                      {status?.cache_misses ?? 0}
                    </Typography>
                  </Box>
                  <Box>
                    <Typography variant="caption" color="text.secondary">Total</Typography>
                    <Typography variant="body1" fontWeight={600}>
                      {status?.total_nodes ?? 0}
                    </Typography>
                  </Box>
                </Stack>
                {cacheStats && (
                  <>
                    <Divider sx={{ my: 2 }} />
                    <Stack spacing={0.5}>
                      <Typography variant="caption" color="text.secondary">
                        Local: {cacheStats.local_cache_dir}
                      </Typography>
                      {cacheStats.remote_url && (
                        <Typography variant="caption" color="text.secondary">
                          Remote: {cacheStats.remote_url}
                        </Typography>
                      )}
                      {cacheStats.regions && (
                        <Typography variant="caption" color="text.secondary">
                          Regions: {cacheStats.regions}
                        </Typography>
                      )}
                    </Stack>
                  </>
                )}
              </CardContent>
            </Card>
          </Grid>

          {/* Workers Card */}
          <Grid item xs={12} md={4}>
            <Card variant="outlined" sx={{ height: '100%' }}>
              <CardContent>
                <Stack direction="row" alignItems="center" spacing={1} sx={{ mb: 2 }}>
                  <CloudIcon color="primary" />
                  <Typography variant="h6">Worker Nodes</Typography>
                </Stack>
                {workers.length === 0 ? (
                  <Box sx={{ textAlign: 'center', py: 3 }}>
                    <StorageIcon sx={{ fontSize: 48, color: 'text.disabled', mb: 1 }} />
                    <Typography variant="body2" color="text.secondary">
                      No remote workers configured.
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      Set MEMOBUILD_WORKERS env var to add workers.
                    </Typography>
                  </Box>
                ) : (
                  <Stack spacing={1}>
                    {workers.map((w) => (
                      <Card key={w.id} variant="outlined" sx={{ p: 1.5 }}>
                        <Stack direction="row" alignItems="center" justifyContent="space-between">
                          <Box>
                            <Typography variant="body2" fontWeight={600}>{w.id}</Typography>
                            <Typography variant="caption" color="text.secondary">{w.url}</Typography>
                          </Box>
                          <Tooltip title={w.healthy ? 'Healthy' : 'Unreachable'}>
                            {w.healthy ? (
                              <CheckCircleIcon color="success" fontSize="small" />
                            ) : (
                              <ErrorIcon color="error" fontSize="small" />
                            )}
                          </Tooltip>
                        </Stack>
                      </Card>
                    ))}
                  </Stack>
                )}
              </CardContent>
            </Card>
          </Grid>

          {/* Last Build Card */}
          <Grid item xs={12} md={4}>
            <Card variant="outlined" sx={{ height: '100%' }}>
              <CardContent>
                <Stack direction="row" alignItems="center" spacing={1} sx={{ mb: 2 }}>
                  <BuildIcon color="primary" />
                  <Typography variant="h6">Last Build</Typography>
                </Stack>
                {status ? (
                  <Stack spacing={1.5}>
                    <Box>
                      <Typography variant="caption" color="text.secondary">Status</Typography>
                      <Box>
                        <Chip
                          label={status.running ? 'Running' : 'Idle'}
                          color={status.running ? 'warning' : 'success'}
                          size="small"
                        />
                      </Box>
                    </Box>
                    <Box>
                      <Typography variant="caption" color="text.secondary">Duration</Typography>
                      <Typography variant="body1" fontWeight={600}>
                        {status.duration_ms}ms
                      </Typography>
                    </Box>
                    <Box>
                      <Typography variant="caption" color="text.secondary">Remote Executor</Typography>
                      <Typography variant="body2" sx={{ wordBreak: 'break-all' }}>
                        {status.remote_exec_url || 'Local (no remote)'}
                      </Typography>
                    </Box>
                  </Stack>
                ) : (
                  <Typography variant="body2" color="text.secondary">No build data yet.</Typography>
                )}
              </CardContent>
            </Card>
          </Grid>

          {/* Build Trigger */}
          <Grid item xs={12}>
            <Card variant="outlined">
              <CardContent>
                <Typography variant="h6" sx={{ mb: 2 }}>
                  Trigger Build
                </Typography>
                <Stack direction={{ xs: 'column', sm: 'row' }} spacing={2} sx={{ mb: 2 }}>
                  <TextField
                    label="Dockerfile path"
                    size="small"
                    value={dockerfile}
                    onChange={(e) => setDockerfile(e.target.value)}
                    sx={{ flex: 1 }}
                    placeholder="Dockerfile"
                  />
                  <TextField
                    label="Remote Executor URL (optional)"
                    size="small"
                    value={remoteExec}
                    onChange={(e) => setRemoteExec(e.target.value)}
                    sx={{ flex: 1 }}
                    placeholder="http://scheduler:9000"
                  />
                  <Button
                    variant="contained"
                    onClick={triggerBuild}
                    disabled={building}
                    startIcon={building ? <CircularProgress size={16} /> : <BuildIcon />}
                    sx={{ minWidth: 140 }}
                  >
                    {building ? 'Building…' : 'Run Build'}
                  </Button>
                </Stack>
                {buildOutput && (
                  <TextField
                    label="Build output"
                    multiline
                    minRows={6}
                    maxRows={16}
                    fullWidth
                    value={buildOutput}
                    InputProps={{ readOnly: true, sx: { fontFamily: 'monospace', fontSize: 12 } }}
                    variant="outlined"
                  />
                )}
              </CardContent>
            </Card>
          </Grid>
        </Grid>
      )}
    </Box>
  );
}
