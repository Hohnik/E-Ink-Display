mod firmware;

fn main() -> anyhow::Result<()> {
    firmware::run()
}
