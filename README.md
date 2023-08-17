# watchdescriptor
`watchdescriptor` is a Rust-based CLN plugin that allows CLN’s bookkeeper plugin to track coin movements in external descriptor wallets, enabling businesses to obtain a complete picture of all bitcoin inflows and outflows.

It utilizes [cln-plugin](https://docs.rs/cln-plugin/latest/cln_plugin/) and the [BDK library](https://github.com/bitcoindevkit/bdk) to track coin movements in registered wallets and report this information to the bookkeeper plugin. 
