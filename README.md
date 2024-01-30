# smaug
`smaug` is a Rust-based CLN plugin that allows CLN’s bookkeeper plugin to track coin movements in external descriptor wallets, enabling businesses to obtain a complete picture of all bitcoin inflows and outflows.

It utilizes [cln-plugin](https://docs.rs/cln-plugin/latest/cln_plugin/) and the [BDK library](https://github.com/bitcoindevkit/bdk) to track coin movements in registered wallets and report this information to the bookkeeper plugin.

This enables businesses to design a complete treasury using [Miniscript](https://bitcoin.sipa.be/miniscript/) and import the resulting descriptor into CLN. Since bookkeeper already accounts for all coin movements internal to CLN, this plugin is the last piece businesses need in order to unify all their bitcoin accounting in one place. This enables businesses to account for all inflows and outflows from their operations, streamlining tax reporting and financial analysis.

## Prerequsites
- [`bitcoind`](https://github.com/bitcoin/bitcoin) with `scanblocks` RPC enabled (at least v24 and `blockfilterindex=1`)
- [`lightningd`](https://github.com/elementsproject/lightning) (aka Core Lightning aka CLN), built from [this branch](https://github.com/niftynei/lightning/tree/nifty/onchain_notif). (This branch adds the two new custom notification topics we use for Smaug: `utxo_deposit` and `utxo_spent`)

I recommend [`nix-bitcoin`](https://nixbitcoin.org) with a config [something like this](https://github.com/chrisguida/nix-dell)
- Obviously this is for mutinynet; if you want mainnet, edit your flake url to pull in the official nix-bitcoin repo)

To get started quickly with nix-bitcoin, I recommend [this excellent tutorial](https://github.com/chrisguida/nixos-mutinynet-tutorial) ;).

## Building
To run Smaug, you will need to build it yourself as there are currently no release binaries. I recommend using the Nix package manager on either Ubuntu or NixOS.

### Nix package manager
(Tested on NixOS and Ubuntu. Mac build is [currently broken](https://github.com/chrisguida/smaug/issues/35). Let me know if you try it on WSL.)

1. Make sure you have a flake-enabled Nix installation. I recommend the [Determinate Systems Nix Installer](https://zero-to-nix.com/start/install) for Ubuntu, Mac, and WSL.
    - Obviously you already have nix installed if you're on NixOS.
2. Enter the following commands:

```
git clone https://github.com/chrisguida/smaug
cd smaug
nix develop
```

This will drop you into a developer shell. Now build the release binary:

```
cargo build --release
```

### Ubuntu and Mac
Install [system dependencies](https://github.com/chrisguida/smaug/blob/master/flake.nix#L41)

Run build:
```
cargo build --release
```

CLN can be a bit challenging to set up and run on Mac. See [this gist](https://gist.github.com/chrisguida/a2adf91dca5787c295f7d59d7d20958c) for hints.

## Installation

On Ubuntu and Mac, you should be able to just run the binary from where it is, or copy it somewhere convenient like `/usr/local/bin` or `$LIGHTNING_DIR/plugins` (where `$LIGHTNING_DIR` is wherever you keep the data for `lightningd`)

On NixOS with nix-bitcoin, you'll need to copy the binary somewhere your lightning user can access. I usually just do:
```
sudo cp target/release/smaug /usr/bin/
```

## Running

To run `smaug`, first make sure `bitcoind` and `lightningd` are both running, and that `bitcoind`'s blockfilterindex has finished syncing.
Then run:
```
lightning-cli plugin -k subcommand=start plugin=</path/to/smaug> smaug_brpc_user=<bitcoind rpc username> smaug_brpc_pass=<bitcoind rpc password>
```

> [!NOTE]
> `smaug_brpc_user` and `smaug_brpc_pass` are currently required.

You may also specify `smaug_brpc_host` and `smaug_brpc_port` to use a custom address.

Also, instead of starting Smaug dynamically, you can start it statically in your lightningd config like so:
```
plugin=/path/to/smaug
smaug_brpc_user=<bitcoind rpc user>
smaug_brpc_pass=<bitcoind rpc password>
```

## Usage
Smaug has a pretty intuitive command line interface which utilizes the very nice `clap` crate:

```
$ lightning-cli smaug

Smaug: a Rust CLN plugin to monitor your treasury

Watch one or more external wallet descriptors and emit notifications when coins are moved

Usage: lightning-cli smaug -- [COMMAND]

Commands:
  add   Start watching a descriptor wallet
  rm    Stop watching a descriptor wallet
  ls    List descriptor wallets currently being watched
  help  Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```
To add a wallet to smaug, just do:
```
lightning-cli smaug add "<external descriptor>" "<internal descriptor>"
```

To try my mutinynet test wallet, do:
```
lightning-cli smaug add "wpkh([56003681/84h/0h/0h]tpubDDrecdRDGxYiu2eduJiPyojMQQJcCnQpyLpA18BkEFtr8S9jgAGAhZ5TKgpArzcnu8qYcVtad2KGXhWsxRgjJbLLwMDH3SW4YcaHbScwLs1/0/*)#840qygs5" "wpkh([56003681/84h/0h/0h]tpubDDrecdRDGxYiu2eduJiPyojMQQJcCnQpyLpA18BkEFtr8S9jgAGAhZ5TKgpArzcnu8qYcVtad2KGXhWsxRgjJbLLwMDH3SW4YcaHbScwLs1/1/*)#kp2peaqv"
```

The command will hang while the wallet is scanning. Depending on your hardware, the scan might take a while. It just takes a couple of minutes on both my mainnet and mutinynet nodes, though.
You can do `watch bitcoin-cli scanblocks status` to see scan progress.

A full scan from genesis to tip is only needed when the wallet is first added. Afterwards, Smaug will simply pick up from where it left off.

As blocks are added, Smaug will report any newly confirmed transactions to CLN's `bookkeeper` plugin.

You can do `lightning-cli help | grep bkpr` to see a list of commands for viewing bookkeeper data.

## Development

We recommend you follow the instructions in [Building](##building) to install Nix and then run `nix develop`. This will drop you into a development shell, with a nice shell hook containing some instructions for how to spin up a regtest bitcoin node and two connected CLN nodes.

To further set up your environment for development we need to install some Python packages from inside the `tests` directory:

#### Install Python packages
```
cd tests
poetry shell
poetry install --no-root
```

#### Install the pre-commit Git hook
```
pre-commit install
```

The [pre-commit](https://pre-commit.com/) Git hook will enforce code formatting and linting on the Python code utilizing the [black](https://black.readthedocs.io/en/stable/), [isort](https://pycqa.github.io/isort/), and [flake8](https://flake8.pycqa.org/en/latest/) packages every time you do a `git commit`. To trigger the hook manually simply run `pre-commit run --all-files`.

### Running the tests
`smaug` is tested with `pytest` using the [pyln-testing](https://pypi.org/project/pyln-testing/) library.
```
pytest
```

> [!NOTE]
> To be able to run the tests remember to first enter your development shell with `nix develop`, change into the the tests directory, and execute `poetry shell` if you haven't yet.

## Feedback
Please [open an issue](https://github.com/chrisguida/smaug/issues/new/choose) if you have any questions or comments!

I can also be reached on Twitter @cguida6.

## Enjoy!
