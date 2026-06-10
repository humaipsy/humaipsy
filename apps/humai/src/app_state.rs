use crate::metrics::MetricsConfig;
use std::sync::RwLock;

struct AppStateInner {
    metrics_config: MetricsConfig,
}

pub struct AppState {
    inner: RwLock<AppStateInner>,
}

impl AppState {
    pub fn new(metrics_config: MetricsConfig) -> Self {
        crate::recording::experiment_csv::update_config(metrics_config.clone());
        Self {
            inner: RwLock::new(AppStateInner { metrics_config }),
        }
    }

    pub fn metrics_config(&self) -> MetricsConfig {
        self.inner.read().unwrap().metrics_config.clone()
    }

    pub fn update_metrics_config(&self, metrics_config: MetricsConfig) {
        crate::recording::experiment_csv::update_config(metrics_config.clone());
        let mut guard = self.inner.write().unwrap();
        guard.metrics_config = metrics_config;
    }
}
