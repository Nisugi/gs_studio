//! The capped status log every tool shares the mechanism of: newest line
//! last, oldest lines dropped past [`STATUS_CAP`]. The stage keeps one, the
//! calibrator tabs keep one; the status bar shows the active tool's.

/// Oldest status lines drop past this.
pub const STATUS_CAP: usize = 50;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StatusLog {
    lines: Vec<String>,
}

impl StatusLog {
    pub fn push(&mut self, message: impl Into<String>) {
        self.lines.push(message.into());
        if self.lines.len() > STATUS_CAP {
            let excess = self.lines.len() - STATUS_CAP;
            self.lines.drain(..excess);
        }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_capped_and_keeps_the_newest() {
        let mut log = StatusLog::default();
        assert!(log.lines().is_empty());
        for n in 0..(STATUS_CAP + 20) {
            log.push(format!("line {n}"));
        }
        assert_eq!(log.lines().len(), STATUS_CAP);
        assert_eq!(log.lines().first().unwrap(), "line 20");
        assert_eq!(log.lines().last().unwrap(), &format!("line {}", STATUS_CAP + 19));
    }
}
