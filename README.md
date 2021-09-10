# simple-pool

Simple and fast async pool for any kind of resources

## The idea

This is a helper library to create custom pools of anything

## Crate

https://crates.io/crates/simple-pool

## Example

```rust
use simple_pool::ResourcePool;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::RwLock;

async fn test() {
	// create local or static resource pool
    let resource_pool: Arc<RwLock<ResourcePool<TcpStream>>> =
        Arc::new(RwLock::new(ResourcePool::new()));
    let mut pool = resource_pool.write().await;
	// put 20 tcp connections there
    for _ in 0..20 {
        let client = TcpStream::connect("127.0.0.1:80").await.unwrap();
        pool.append(client);
    }
    drop(pool);
    let mut fut = Vec::new();
    let started = std::time::Instant::now();
    let n = 1_000_000;
    for _ in 0..n {
        let res_pool = resource_pool.clone();
        fut.push(tokio::spawn(async move {
            // gets one of 20 open tcp connections as soon as one is available
            let _client = res_pool.read().await.get().await;
        }));
    }
}
```
