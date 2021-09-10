//! # simple-pool
//! 
//! Simple and fast async pool for any kind of resources
//! 
//! ## The idea
//! 
//! This is a helper library to create custom pools of anything
//! 
//! ## Crate
//! 
//! <https://crates.io/crates/simple-pool>
//! 
//! ## Example
//! 
//! ```rust
//! use simple_pool::ResourcePool;
//! use std::sync::Arc;
//! use tokio::net::TcpStream;
//! use tokio::sync::RwLock;
//! 
//! async fn test() {
//!     // create a local or static resource pool
//!     let resource_pool: Arc<RwLock<ResourcePool<TcpStream>>> =
//!         Arc::new(RwLock::new(ResourcePool::new()));
//!     {
//!         let mut pool = resource_pool.write().await;
//!         // put 20 tcp connections there
//!         for _ in 0..20 {
//!             let client = TcpStream::connect("127.0.0.1:80").await.unwrap();
//!             pool.append(client);
//!         }
//!     }
//!     let n = 1_000_000;
//!     for _ in 0..n {
//!         let res_pool = resource_pool.clone();
//!         tokio::spawn(async move {
//!             // gets open tcp connection as soon as one is available
//!             let _client = res_pool.read().await.get().await;
//!         });
//!     }
//! }
//! ```
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

struct ResourcePoolGet<'a, T> {
    pool: &'a ResourcePool<T>,
}

impl<'a, T> Future for ResourcePoolGet<'a, T> {
    type Output = ResourcePoolGuard<T>;
    fn poll(self: Pin<&mut ResourcePoolGet<'a, T>>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut holder = self.pool.holder.lock().unwrap();
        holder.resources.pop().map_or_else(
            || {
                holder.append_callback(cx.waker().clone());
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

struct ResourceHolder<T> {
    resources: Vec<T>,
    wakers: Vec<Waker>,
}

impl<T> ResourceHolder<T> {
    fn new(size: usize) -> Self {
        Self {
            resources: Vec::with_capacity(size),
            wakers: Vec::new(),
        }
    }

    fn append_resource(&mut self, res: T) {
        self.resources.push(res);
        while !self.wakers.is_empty() {
            self.wakers.remove(0).wake();
        }
    }

    fn append_callback(&mut self, waker: Waker) {
        self.wakers.push(waker);
    }
}

/// Versatile resource pool
pub struct ResourcePool<T> {
    holder: Arc<Mutex<ResourceHolder<T>>>,
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
    /// thread.
    pub fn append(&mut self, res: T) {
        let mut resources = self.holder.lock().unwrap();
        resources.append_resource(res);
    }

    /// Get resource from the pool or wait until one is available
    pub async fn get(&self) -> ResourcePoolGuard<T> {
        self._get().await
    }

    fn _get(&self) -> ResourcePoolGet<T> {
        ResourcePoolGet { pool: self }
    }
}

/// Returns a reference to the original resource
pub struct ResourcePoolGuard<T> {
    resource: Option<T>,
    holder: Arc<Mutex<ResourceHolder<T>>>,
    need_return: bool,
}

impl<T> ResourcePoolGuard<T> {
    /// Do not return resource back to the pool when dropped
    pub fn forget_resource(&mut self) {
        self.need_return = false;
    }
    /// Replace resource with a new one
    pub fn replace_resource(&mut self, resource: T) {
        self.need_return = true;
        self.resource.replace(resource);
    }
}

impl<T> Deref for ResourcePoolGuard<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.resource.as_ref().unwrap()
    }
}

impl<T> DerefMut for ResourcePoolGuard<T> {
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
