# process-reaper
A program to lethally enforce memory limits on leaky processes. I made this as a quick fix to VSCode's `cpptools` going insane, eating up all my system's RAM, and causing a dang lock up.

I've only tested on Fedora 39, so use at your risk other places.

## Building
Its all Rust, so just do:
```
cargo build --release
```

Executable will be at:
```
target/release/process-reaper
```

## As a Systemd Daemon
There's an example `.service` file in `systemd/`. Modify for your own needs. Make sure to include the argument `--systemd-notify` if using `Type=notify`. 

## Example Use
Kill `cpptools` if it eats up more than 10 GiB of RAM:
```
process-reaper -p cpptools -m 10GiB
```

For all options, simply run:
```
process-reaper --help
```
