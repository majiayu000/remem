pub(super) struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
}

pub(super) enum Status {
    Ok,
    Warn,
    Fail,
}

impl Check {
    pub(super) fn icon(&self) -> &'static str {
        match self.status {
            Status::Ok => "ok",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
        }
    }
}
