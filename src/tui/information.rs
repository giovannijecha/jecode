pub(super) struct Information {
    pub title: String,
    pub details: Vec<(String, String)>,
    pub offset: usize,
}

impl Information {
    pub fn new(title: String, details: Vec<(String, String)>) -> Self {
        Self {
            title,
            details,
            offset: 0,
        }
    }
}
