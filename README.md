# simple-pool

Simple and fast async pool for any kind of resources

## The idea

This is a helper library to create custom pools of anything

## Crate

<https://crates.io/crates/simple-pool>

## Example

```rust,ignore
use simple_pool::ResourcePool;
use std::sync::Arc;
use tokio::net::TcpStream;

async fn test() {
    // create a local or static resource pool
    let resource_pool: Arc<ResourcePool<TcpStream>> =
        Arc::new(ResourcePool::new());
    {
        // put 20 tcp connections there
        for _ in 0..20 {
            let client = TcpStream::connect("127.0.0.1:80").await.unwrap();
            resource_pool.append(client);
        }
    }
    let n = 1_000_000;
    for _ in 0..n {
        let pool = resource_pool.clone();
        tokio::spawn(async move {
            // gets open tcp connection as soon as one is available
            let _client = pool.get().await;
        });
    }
}
```
