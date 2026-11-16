use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustodyLimits {
    pub records: usize,
    pub bytes: usize,
    pub age_seconds: f64,
    pub chats: usize,
    pub frame_bytes: usize,
    pub operation_seconds: f64,
    pub tasks: usize,
}
impl CustodyLimits {
    pub fn new(records: usize, bytes: usize, age_seconds: f64) -> Result<Self> {
        let value = Self {
            records,
            bytes,
            age_seconds,
            chats: 64,
            frame_bytes: 262144,
            operation_seconds: 15.0,
            tasks: 8,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        for (value, max) in [
            (self.records, 4096),
            (self.bytes, 64 * 1024 * 1024),
            (self.chats, 64),
            (self.frame_bytes, 1024 * 1024),
            (self.tasks, 64),
        ] {
            if value == 0 || value > max {
                return Err(Error::Capacity);
            }
        }
        for (value, max) in [
            (self.age_seconds, 30.0 * 86400.0),
            (self.operation_seconds, 60.0),
        ] {
            if !value.is_finite() || value <= 0.0 || value > max {
                return Err(Error::Capacity);
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransientLimits {
    pub chats: usize,
    pub records: usize,
    pub bytes: usize,
    pub frame_bytes: usize,
    pub retention_seconds: f64,
    pub operation_seconds: f64,
    pub cleanup_seconds: f64,
    pub tasks: usize,
    pub attempts: u8,
}
impl Default for TransientLimits {
    fn default() -> Self {
        Self {
            chats: 64,
            records: 512,
            bytes: 8 * 1024 * 1024,
            frame_bytes: 262144,
            retention_seconds: 300.0,
            operation_seconds: 15.0,
            cleanup_seconds: 5.0,
            tasks: 8,
            attempts: 3,
        }
    }
}
impl TransientLimits {
    pub fn validate(&self) -> Result<()> {
        CustodyLimits {
            records: self.records,
            bytes: self.bytes,
            age_seconds: self.retention_seconds,
            chats: self.chats,
            frame_bytes: self.frame_bytes,
            operation_seconds: self.operation_seconds,
            tasks: self.tasks,
        }
        .validate()?;
        if self.frame_bytes > self.bytes
            || self.retention_seconds > 3600.0
            || !self.cleanup_seconds.is_finite()
            || self.cleanup_seconds <= 0.0
            || self.cleanup_seconds > 30.0
            || self.attempts == 0
            || self.attempts > 10
        {
            return Err(Error::Capacity);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IngressLimits {
    pub records: usize,
    pub bytes: usize,
    pub age_seconds: f64,
    pub batch_bytes: usize,
    pub operation_seconds: f64,
}
impl IngressLimits {
    pub fn new(records: usize, bytes: usize, age_seconds: f64) -> Result<Self> {
        let value = Self {
            records,
            bytes,
            age_seconds,
            batch_bytes: 1024 * 1024,
            operation_seconds: 15.0,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        CustodyLimits::new(self.records, self.bytes, self.age_seconds)?.validate()?;
        if self.batch_bytes == 0
            || self.batch_bytes > 4 * 1024 * 1024
            || self.batch_bytes > self.bytes
            || !self.operation_seconds.is_finite()
            || self.operation_seconds <= 0.0
            || self.operation_seconds > 60.0
        {
            return Err(Error::Capacity);
        }
        Ok(())
    }
}
