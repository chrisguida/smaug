# watchdescriptor
`watchdescriptor` is a Rust-based CLN plugin that allows CLN’s bookkeeper plugin to track coin movements in external descriptor wallets, enabling businesses to obtain a complete picture of all bitcoin inflows and outflows.

It utilizes [cln-plugin](https://docs.rs/cln-plugin/latest/cln_plugin/) and the [BDK library](https://github.com/bitcoindevkit/bdk) to track coin movements in registered wallets and report this information to the bookkeeper plugin. 

This enables businesses to design a complete treasury using [Miniscript](https://bitcoin.sipa.be/miniscript/) and import the resulting descriptor into CLN. Since bookkeeper already accounts for all coin movements internal to CLN, this plugin is the last piece businesses need in order to unify all their bitcoin accounting in one place. This enables businesses to account for all inflows and outflows from their operations, streamlining tax reporting and financial analysis.
