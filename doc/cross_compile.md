# Cross compile

Add to `~/.cargo/config.toml`

```
[target.riscv64gc-unknown-linux-gnu]
linker = "riscv64-linux-gnu-gcc"
```

```
sudo apt-get install gcc-riscv64-linux-gnu
rustup target add riscv64gc-unknown-linux-gnu
cargo +nightly build -Zbuild-std --release --target riscv64gc-unknown-linux-gnu --example ax25-1200-rx
```
