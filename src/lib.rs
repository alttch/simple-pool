#![ doc = include_str!( concat!( env!( "CARGO_MANIFEST_DIR" ), "/", "README.md" ) ) ]
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::{mpsc, Arc, Mutex};
use std::task::{Context, Poll, Waker};

struct ResourcePoolGet<'a, T> {
    pool: &'a ResourcePool<T>,
    alive: Mutex<Option<mpsc::Receiver<()>>>,
}

impl<'a, T> Future for ResourcePoolGet<'a, T> {
    type Output = ResourcePoolGuard<T>;
    fn poll(self: Pin<&mut ResourcePoolGet<'a, T>>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut holder = self.pool.holder.lock().unwrap();
        holder.resources.pop().map_or_else(
            || {
                let (tx, rx) = mpsc::channel();
                self.alive.lock().unwrap().replace(rx);
                holder.append_callback(cx.waker().clone(), tx);
                Poll::Pending
            },
            |res| {
                Poll::Ready(ResourcePoolGuard {
                    resource: Some(res),
                    holder: self.pool.holder.clone(),
                    need_return: true,
                })
            },
        )
    }
}

/// Access directly only if you know what you are doing
pub struct ResourceHolder<T> {
    pub resources: Vec<T>,
    wakers: Vec<(Waker, mpsc::Sender<()>)>,
}

impl<T> ResourceHolder<T> {
    fn new(size: usize) -> Self {
        Self {
            resources: Vec::with_capacity(size),
            wakers: <_>::default(),
        }
    }

    #[inline]
    fn append_resource(&mut self, res: T) {
        self.resources.push(res);
        while !self.wakers.is_empty() {
            let (waker, tx) = self.wakers.remove(0);
            if tx.send(()).is_ok() {
                waker.wake();
                break;
            }
        }
    }

    #[inline]
    fn append_callback(&mut self, waker: Waker, tx: mpsc::Sender<()>) {
        self.wakers.push((waker, tx));
    }
}

/// Versatile resource pool
pub struct ResourcePool<T> {
    pub holder: Arc<Mutex<ResourceHolder<T>>>,
}

impl<T> Default for ResourcePool<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ResourcePool<T> {
    /// Create new resource pool
    pub fn new() -> Self {
        Self {
            holder: Arc::new(Mutex::new(ResourceHolder::new(0))),
        }
    }
    /// Create new resource pool with pre-allocated capacity
    ///
    /// The size parameter is used to pre-allocate memory for the resource holder only
    pub fn with_capacity(size: usize) -> Self {
        Self {
            holder: Arc::new(Mutex::new(ResourceHolder::new(size))),
        }
    }

    /// Append resource to the pool
    ///
    /// # Panics
    ///
    /// This function might panic when called if the resource lock is already held by the current
    /// thread
    #[inline]
    pub fn append(&self, res: T) {
        let mut resources = self.holder.lock().unwrap();
        resources.append_resource(res);
    }

    /// Get resource from the pool or wait until one is available
    #[inline]
    pub async fn get(&self) -> ResourcePoolGuard<T> {
        self._get().await
    }
    #[inline]
    fn _get(&self) -> ResourcePoolGet<T> {
        ResourcePoolGet {
            pool: self,
            alive: <_>::default(),
        }
    }
}

/// Returns a container with a resource
///
/// When dropped, the resource is sent back to the pool
pub struct ResourcePoolGuard<T> {
    resource: Option<T>,
    holder: Arc<Mutex<ResourceHolder<T>>>,
    need_return: bool,
}

impl<T> ResourcePoolGuard<T> {
    /// Do not return resource back to the pool when dropped
    #[inline]
    pub fn forget_resource(&mut self) {
        self.need_return = false;
    }
    /// Replace resource with a new one
    #[inline]
    pub fn replace_resource(&mut self, resource: T) {
        self.need_return = true;
        self.resource.replace(resource);
    }
}

impl<T> Deref for ResourcePoolGuard<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.resource.as_ref().unwrap()
    }
}

impl<T> DerefMut for ResourcePoolGuard<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.resource.as_mut().unwrap()
    }
}

impl<T> Drop for ResourcePoolGuard<T> {
    fn drop(&mut self) {
        if self.need_return {
            self.holder
                .lock()
                .unwrap()
                .append_resource(self.resource.take().unwrap());
        }
    }
}
