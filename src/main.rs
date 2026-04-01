use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    remem::cli::run().await
}
