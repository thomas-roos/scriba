//! Circular audio buffer for voice detection pre-buffering.

use std::collections::VecDeque;

/// A fixed-capacity circular buffer for f32 audio samples.
///
/// Used to keep a rolling window of recent audio so that speech
/// immediately preceding a wake phrase can be captured.
pub struct RingBuffer {
    buffer: VecDeque<f32>,
    capacity: usize,
}

impl RingBuffer {
    /// Create a new ring buffer with the given capacity in samples.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push new samples into the buffer, discarding oldest samples if full.
    pub fn push_samples(&mut self, samples: &[f32]) {
        for &sample in samples {
            if self.buffer.len() >= self.capacity {
                self.buffer.pop_front();
            }
            self.buffer.push_back(sample);
        }
    }

    /// Drain all samples from the buffer, returning them in order.
    pub fn drain_all(&mut self) -> Vec<f32> {
        self.buffer.drain(..).collect()
    }

    /// Current number of samples in the buffer.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_drain() {
        let mut buf = RingBuffer::new(5);
        buf.push_samples(&[1.0, 2.0, 3.0]);
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.drain_all(), vec![1.0, 2.0, 3.0]);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_overflow_discards_oldest() {
        let mut buf = RingBuffer::new(4);
        buf.push_samples(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(buf.len(), 4);
        assert_eq!(buf.drain_all(), vec![3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_empty_buffer() {
        let mut buf = RingBuffer::new(10);
        assert!(buf.is_empty());
        assert_eq!(buf.drain_all(), Vec::<f32>::new());
    }
}
