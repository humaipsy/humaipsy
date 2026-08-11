use humai_core::metrics::MetricsConfig;
use humai_core::sync_ext::RwLockExt;
use std::sync::RwLock;

struct AppStateInner {
    metrics_config: MetricsConfig,
}

pub struct AppState {
    inner: RwLock<AppStateInner>,
}

impl AppState {
    pub fn new(metrics_config: MetricsConfig) -> Self {
        humai_core::recording::experiment_csv::update_config(metrics_config.clone());
        Self {
            inner: RwLock::new(AppStateInner { metrics_config }),
        }
    }

    pub fn metrics_config(&self) -> MetricsConfig {
        self.inner.read_recover().metrics_config.clone()
    }

    pub fn update_metrics_config(&self, metrics_config: MetricsConfig) {
        humai_core::recording::experiment_csv::update_config(metrics_config.clone());
        let mut guard = self.inner.write_recover();
        guard.metrics_config = metrics_config;
    }
}
