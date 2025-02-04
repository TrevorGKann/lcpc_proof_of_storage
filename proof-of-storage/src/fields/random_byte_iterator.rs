use rand::rngs::ThreadRng;
use rand_core::RngCore;

pub struct RandomBytesIterator {
    byte_buffer: [u8; 8192],
    position: usize,
    rng: ThreadRng,
}

impl RandomBytesIterator {
    pub fn new() -> Self {
        Self {
            byte_buffer: [0; 8192],
            position: 0,
            rng: rand::thread_rng(),
        }
    }
}

impl Iterator for RandomBytesIterator {
    type Item = u8;
    fn next(&mut self) -> Option<Self::Item> {
        if self.position >= self.byte_buffer.len() {
            self.position = 0;
        }

        if self.position == 0 {
            self.rng.fill_bytes(&mut self.byte_buffer);
        }

        self.position += 1;
        Some(self.byte_buffer[self.position - 1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn can_i_use_take_a_billion() {
        let iterator = RandomBytesIterator::new();
        let mut test = iterator.take(usize::max_value());
        let _next = test.next();
    }
}
