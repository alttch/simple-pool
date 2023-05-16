#![ doc = include_str!( concat!( env!( "CARGO_MANIFEST_DIR" ), "/", "README.md" ) ) ]
use parking_lot::Mutex;
use std::collections::BTreeSet;
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::ptr::addr_of;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

type ClientId = usize;

struct ResourcePoolGet<'a, T> {
    pool: &'a ResourcePool<T>,
    queued: bool,
}

impl<'a, T> Future for ResourcePoolGet<'a, T> {
    type Output = ResourcePoolGuard<T>;
    fn poll(
        mut self: Pin<&mut ResourcePoolGet<'a, T>>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let mut holder = self.pool.holder.lock();
        // there are no other futures waiting or we are queued and started from a waker
        if holder.wakers.is_empty() || self.queued {
            if let Some(res) = holder.resources.pop() {
                self.queued = false;
                // notify the pool the resource is taken and we were not aborted
                holder.confirm_get(self.id());
                return Poll::Ready(ResourcePoolGuard {
                    resource: Some(res),
                    holder: self.pool.holder.clone(),
                });
            }
        }
        self.queued = true;
        holder.append_callback(cx.waker().clone(), self.id());
        Poll::Pending
    }
}

impl<'a, T> ResourcePoolGet<'a, T> {
    #[inline]
    fn id(&self) -> ClientId {
        // as ID is used only when the object is alive, it is pretty safe to use pointer of queued
        // as the unique object id
        addr_of!(self.queued).cast::<bool>() as ClientId
    }
}

impl<'a, T> Drop for ResourcePoolGet<'a, T> {
    #[inline]
    fn drop(&mut self) {
        // notify the pool we are dropped
        self.pool.holder.lock().notify_drop(self.id());
    }
}

/// Access directly only if you know what you are doing
pub struct ResourceHolder<T> {
    pub resources: Vec<T>,
    wakers: Vec<(Waker, ClientId)>,
    pending: BTreeSet<ClientId>,
}

impl<T> ResourceHolder<T> {
    fn new(size: usize) -> Self {
        Self {
            resources: Vec::with_capacity(size),
            wakers: <_>::default(),
            pending: <_>::default(),
        }
    }

    #[inline]
    fn append_resource(&mut self, res: T) {
        self.resources.push(res);
        self.wake_next();
    }

    #[inline]
    fn wake_next(&mut self) {
        if !self.wakers.is_empty() {
            let (waker, id) = self.wakers.remove(0);
            self.pending.insert(id);
            waker.wake();
        }
    }

    #[inline]
    fn notify_drop(&mut self, id: ClientId) {
        // remove the future from wakers
        if let Some(pos) = self.wakers.iter().position(|(_, i)| *i == id) {
            self.wakers.remove(pos);
        }
        // if the future was pending to get a resource, wake the next one
        if self.pending.remove(&id) {
            self.wake_next();
        }
    }

    #[inline]
    fn confirm_get(&mut self, id: ClientId) {
        // the resource is taken, remove from pending
        self.pending.remove(&id);
    }

    #[inline]
    fn append_callback(&mut self, waker: Waker, id: ClientId) {
        self.wakers.push((waker, id));
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
    /// Create a new resource pool
    pub fn new() -> Self {
        Self {
            holder: Arc::new(Mutex::new(ResourceHolder::new(0))),
        }
    }
    /// Create a new resource pool with pre-allocated capacity
    ///
    /// The size parameter is used to pre-allocate memory for the resource holder only
    pub fn with_capacity(size: usize) -> Self {
        Self {
            holder: Arc::new(Mutex::new(ResourceHolder::new(size))),
        }
    }

    /// Append a resource to the pool
    #[inline]
    pub fn append(&self, res: T) {
        let mut resources = self.holder.lock();
        resources.append_resource(res);
    }

    /// Get a resource from the pool or wait until one is available
    #[inline]
    pub fn get(&self) -> impl Future<Output = ResourcePoolGuard<T>> + '_ {
        ResourcePoolGet {
            pool: self,
            queued: false,
        }
    }
}

/// Returns a container with a resource
///
/// When dropped, the resource is sent back to the pool
pub struct ResourcePoolGuard<T> {
    resource: Option<T>,
    holder: Arc<Mutex<ResourceHolder<T>>>,
}

impl<T> ResourcePoolGuard<T> {
    /// Do not return resource back to the pool when dropped
    #[inline]
    pub fn forget_resource(&mut self) {
        self.resource.take();
    }
    /// Replace resource with a new one
    #[inline]
    pub fn replace_resource(&mut self, resource: T) {
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
        if let Some(res) = self.resource.take() {
            self.holder.lock().append_resource(res);
        }
    }
}

#[cfg(test)]
// the tests test the pool for various problems and may go pretty long
mod test {
    use super::ResourcePool;
    use std::sync::Arc;
    use std::time::Duration;
    use std::time::Instant;
    use tokio::sync::mpsc;
    use tokio::time::sleep;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ordering() {
        for _ in 0..5 {
            let pool = Arc::new(ResourcePool::new());
            let op = Instant::now();
            pool.append(());
            let n = 1_000;
            let mut futs = Vec::new();
            let (tx, mut rx) = mpsc::channel(n);
            for i in 1..=n {
                let p = pool.clone();
                let tx = tx.clone();
                let fut = tokio::spawn(async move {
                    sleep(Duration::from_millis(1)).await;
                    //println!("future {} started {}", i, op.elapsed().as_millis());
                    let _lock = p.get().await;
                    tx.send(i).await.unwrap();
                    println!("future {} locked {}", i, op.elapsed().as_millis());
                    sleep(Duration::from_millis(10)).await;
                    //println!("future {} finished", i);
                });
                sleep(Duration::from_millis(2)).await;
                if i > 1 && (i - 2) % 10 == 0 {
                    println!("future {} canceled", i);
                    fut.abort();
                } else {
                    futs.push(fut);
                }
            }
            for fut in futs {
                tokio::time::timeout(Duration::from_secs(10), fut)
                    .await
                    .unwrap()
                    .unwrap();
            }
            let mut i = 0;
            loop {
                i += 1;
                if i > 1 && (i - 2) % 10 == 0 {
                    i += 1;
                }
                if i > n {
                    break;
                }
                let fut_n = rx.recv().await.unwrap();
                assert_eq!(i, fut_n);
            }
            assert!(
                pool.holder.lock().pending.is_empty(),
                "pool is poisoned (pendings)",
            );
        }
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_no_poisoning() {
        let n = 2_000;
        for i in 1..=n {
            let pool: Arc<ResourcePool<()>> = Arc::new(ResourcePool::new());
            let pool_c = pool.clone();
            let fut1 = tokio::spawn(async move {
                sleep(Duration::from_millis(1)).await;
                let _resource = pool_c.get().await;
                //println!("fut1 completed");
            });
            let pool_c = pool.clone();
            let _fut2 = tokio::spawn(async move {
                sleep(Duration::from_millis(2)).await;
                let _resource = pool_c.get().await;
                //println!("fut2 completed");
            });
            let pool_c = pool.clone();
            let _fut3 = tokio::spawn(async move {
                sleep(Duration::from_millis(3)).await;
                let _resource = pool_c.get().await;
                //println!("fut3 completed");
            });
            sleep(Duration::from_millis(2)).await;
            if i % 2 == 0 {
                pool.append(());
                fut1.abort();
            } else {
                fut1.abort();
                pool.append(());
            }
            sleep(Duration::from_millis(10)).await;
            let holder = pool.holder.lock();
            assert!(
                holder.wakers.is_empty(),
                "pool is poisoned {}/{}",
                holder.wakers.len(),
                holder.resources.len()
            );
            assert!(holder.pending.is_empty(), "pool is poisoned (pendings)",);
        }
    }
}
