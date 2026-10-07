#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", choruz_computer_use::manage(None, None).await?);
    Ok(())
}
