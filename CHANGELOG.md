# Changelog

## [0.1.4] 2026-01-21

### Added

- Bitcoind RPC credential auto-detection with 6-priority fallback chain
- Graceful startup mode: plugin now starts without credentials and returns helpful error on RPC calls
- New `src/brpc_auth.rs` module for credential detection logic
- Support for reading credentials from CLN's `listconfigs` RPC
- Support for parsing `~/.bitcoin/bitcoin.conf` with network-specific sections
- Comprehensive tests for credential detection scenarios

### Fixed

- Plugin no longer crashes when bitcoind credentials are not configured
- Correct cookie file paths for all networks (mainnet, testnet, regtest, signet)

## [0.1.3] 2026-01-21

### Changed

- (tests) move python test deps to tests/ and switch to uv for reckless support

## [0.1.2] 2026-01-21

### Changed

- upgrade cln-rpc and cln-plugin dependencies to 0.3
- (tests) fix compatibility with CLN 25.x, Bitcoin Core 28.1+, and upstream plugins repo CI

## [0.1.1] 2025-01-31

### Fixed

- better handling of cookie file on signet/mutinynet
- use default port for signet/mutinynet

## [0.1.0] 2024-09-27

### Added

- initial release
