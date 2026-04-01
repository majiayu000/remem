pub struct Timer {
    component: String,
    start: std::time::Instant,
}

impl Timer {
    pub fn start(component: &str, msg: &str) -> Self {
        crate::log::info(component, &format!("START {}", msg));
        Self {
            component: component.to_string(),
            start: std::time::Instant::now(),
        }
    }

    pub fn done(self, msg: &str) {
        let ms = self.start.elapsed().as_millis();
        crate::log::info(&self.component, &format!("DONE {}ms {}", ms, msg));
    }
}
