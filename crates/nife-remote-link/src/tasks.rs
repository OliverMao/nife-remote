//! Owned async task: dropping the owner cancels the task, including on error,
//! timeout and parent cancellation. A bare JoinHandle would detach instead.
use std::ops::{Deref, DerefMut};
use tokio::task::JoinHandle;

pub struct OwnedTask<T>(pub JoinHandle<T>);
impl<T> Drop for OwnedTask<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl<T> Deref for OwnedTask<T> {
    type Target = JoinHandle<T>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T> DerefMut for OwnedTask<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}


