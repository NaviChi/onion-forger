use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    let mut tasks = Vec::new();
    tasks.push(tokio::spawn(async {
        let _ = tokio::time::timeout(tokio::time::Duration::from_secs(1), async {
            println!("Task 0 fast start");
            sleep(Duration::from_millis(500)).await;
            println!("Task 0 fast end");
        }).await;
    }));
    tasks.push(tokio::spawn(async {
        println!("Task 1 hanging");
        std::thread::sleep(std::time::Duration::from_secs(8));
        println!("Task 1 end hanging");
    }));
    
    println!("Awaiting select_all");
    let res = futures::future::select_all(tasks).await;
    println!("select_all returned early! index={}", res.1);
}
