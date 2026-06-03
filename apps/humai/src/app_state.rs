use crate::metrics::{MetricsConfig, MetricsSink, build_metrics_sink};
use std::sync::{Arc, RwLock};

struct AppStateInner {
    metrics_config: MetricsConfig,
    metrics_sink: Arc<dyn MetricsSink>,
}

pub struct AppState {
    inner: RwLock<AppStateInner>,
}

impl AppState {
    pub fn new(metrics_config: MetricsConfig) -> Self {
        let metrics_sink = build_metrics_sink(&metrics_config);
        Self {
            inner: RwLock::new(AppStateInner {
                metrics_config,
                metrics_sink,
            }),
        }
    }

    pub fn metrics_config(&self) -> MetricsConfig {
        self.inner.read().unwrap().metrics_config.clone()
    }

    pub fn metrics_sink(&self) -> Arc<dyn MetricsSink> {
        self.inner.read().unwrap().metrics_sink.clone()
    }

    pub fn update_metrics_config(&self, metrics_config: MetricsConfig) {
        let metrics_sink = build_metrics_sink(&metrics_config);
        let mut guard = self.inner.write().unwrap();
        guard.metrics_config = metrics_config;
        guard.metrics_sink = metrics_sink;
    }
}
