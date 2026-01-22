"""
Tests for bitcoind RPC credential auto-detection.

This file contains two types of tests:
1. pyln-testing tests: Use the standard pyln fixtures (node_factory, bitcoind)
2. Manual tests: Use subprocess to run real bitcoind/lightningd instances
   - Run with: pytest -v -s -m manual
   - These are slower but test scenarios pyln-testing can't handle

Detection priority order:
1. Explicit smaug_brpc_user + smaug_brpc_pass
2. Explicit smaug_brpc_cookie_dir
3. listconfigs RPC for bitcoin-rpc* options
4. Auto-detect cookie at standard paths
5. Parse ~/.bitcoin/bitcoin.conf
6. Graceful startup with warning
"""

import os
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

import pytest
from pyln.testing.fixtures import *  # noqa: F401, F403
from pyln.testing.utils import BITCOIND_CONFIG, reserve_unused_port

# Define utility paths
RUST_PROFILE = os.environ.get("RUST_PROFILE", "debug")
PROJECT_ROOT = Path(__file__).parent.parent
COMPILED_PATH = PROJECT_ROOT / "target" / RUST_PROFILE / "smaug"
DOWNLOAD_PATH = Path(__file__).parent / "smaug"


def get_plugin():
    if COMPILED_PATH.is_file():
        return COMPILED_PATH
    elif DOWNLOAD_PATH.is_file():
        return DOWNLOAD_PATH
    else:
        raise ValueError("No plugin was found.")


# =============================================================================
# pyln-testing based tests
# =============================================================================


def test_explicit_smaug_config(node_factory, bitcoind):
    """Priority 1: Explicit smaug_brpc_user + smaug_brpc_pass works"""
    opts = {
        "plugin": get_plugin(),
        "smaug_brpc_user": BITCOIND_CONFIG["rpcuser"],
        "smaug_brpc_pass": BITCOIND_CONFIG["rpcpassword"],
        "smaug_brpc_port": BITCOIND_CONFIG["rpcport"],
    }
    l1 = node_factory.get_node(options=opts)

    result = l1.rpc.call("smaug", ["ls"])

    assert isinstance(result, dict)
    assert "error" not in result or "not_configured" not in result.get(
        "error", ""
    )


def test_listconfigs_fallback(node_factory, bitcoind):
    """Priority 3: Falls back to bitcoin-rpc* from listconfigs

    When no explicit smaug_brpc_* options are set, smaug should fall back
    to reading bitcoin-rpcuser/bitcoin-rpcpassword from CLN's listconfigs.
    """
    opts = {
        "plugin": get_plugin(),
        # No smaug_brpc_* options - should fall back to listconfigs
    }
    l1 = node_factory.get_node(options=opts)

    result = l1.rpc.call("smaug", ["ls"])

    assert isinstance(result, dict)
    assert "error" not in result or "not_configured" not in result.get(
        "error", ""
    )


# =============================================================================
# Manual tests (run with: pytest -v -s -m manual)
# These spin up real bitcoind/lightningd instances via subprocess
# =============================================================================


def wait_for_bitcoind(datadir, rpcport=None, timeout=30):
    """Wait for bitcoind to be ready."""
    start = time.time()
    while time.time() - start < timeout:
        try:
            cmd = ["bitcoin-cli", "-regtest", f"-datadir={datadir}"]
            if rpcport:
                cmd.append(f"-rpcport={rpcport}")
            cmd.append("getblockchaininfo")
            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=5,
            )
            if result.returncode == 0:
                return True
        except subprocess.TimeoutExpired:
            pass
        time.sleep(0.5)
    return False


def wait_for_lightningd(lightning_dir, timeout=30):
    """Wait for lightningd to be ready."""
    start = time.time()
    while time.time() - start < timeout:
        try:
            result = subprocess.run(
                [
                    "lightning-cli",
                    f"--lightning-dir={lightning_dir}",
                    "--network=regtest",
                    "getinfo",
                ],
                capture_output=True,
                text=True,
                timeout=5,
            )
            if result.returncode == 0:
                return True
        except subprocess.TimeoutExpired:
            pass
        time.sleep(0.5)
    return False


def stop_bitcoind(datadir, rpcport=None):
    """Stop bitcoind gracefully."""
    try:
        cmd = ["bitcoin-cli", "-regtest", f"-datadir={datadir}"]
        if rpcport:
            cmd.append(f"-rpcport={rpcport}")
        cmd.append("stop")
        subprocess.run(
            cmd,
            capture_output=True,
            timeout=10,
        )
        time.sleep(1)
    except Exception:
        pass


def stop_lightningd(lightning_dir):
    """Stop lightningd gracefully."""
    try:
        subprocess.run(
            [
                "lightning-cli",
                f"--lightning-dir={lightning_dir}",
                "--network=regtest",
                "stop",
            ],
            capture_output=True,
            timeout=10,
        )
        time.sleep(1)
    except Exception:
        pass


@pytest.fixture
def manual_test_dirs():
    """Create temp directories and allocate unique ports for manual tests."""
    bitcoin_dir = tempfile.mkdtemp(prefix="smaug-test-bitcoin-")
    lightning_dir = tempfile.mkdtemp(prefix="smaug-test-lightning-")
    rpcport = reserve_unused_port()
    p2p_port = reserve_unused_port()
    ln_port = reserve_unused_port()

    yield {
        "bitcoin": bitcoin_dir,
        "lightning": lightning_dir,
        "rpcport": rpcport,
        "p2p_port": p2p_port,
        "ln_port": ln_port,
    }

    # Cleanup
    stop_lightningd(lightning_dir)
    stop_bitcoind(bitcoin_dir, rpcport)
    time.sleep(1)
    shutil.rmtree(bitcoin_dir, ignore_errors=True)
    shutil.rmtree(lightning_dir, ignore_errors=True)


@pytest.mark.manual
def test_cookie_file_auth(manual_test_dirs):
    """Priority 2: Cookie file authentication works.

    Start bitcoind without rpcuser/rpcpassword (cookie auth),
    then verify smaug can connect using smaug_brpc_cookie_dir.
    """
    bitcoin_dir = manual_test_dirs["bitcoin"]
    lightning_dir = manual_test_dirs["lightning"]
    rpcport = manual_test_dirs["rpcport"]
    p2p_port = manual_test_dirs["p2p_port"]
    ln_port = manual_test_dirs["ln_port"]
    log_file = Path(lightning_dir) / "lightningd.log"

    # Start bitcoind without rpcuser/rpcpassword (uses cookie auth)
    print(f"\nStarting bitcoind (cookie auth) in {bitcoin_dir}")
    subprocess.Popen(
        [
            "bitcoind",
            "-regtest",
            f"-datadir={bitcoin_dir}",
            f"-rpcport={rpcport}",
            f"-port={p2p_port}",
            "-fallbackfee=0.0001",
            "-daemon",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    assert wait_for_bitcoind(bitcoin_dir, rpcport), "bitcoind failed to start"

    cookie_path = Path(bitcoin_dir) / "regtest" / ".cookie"
    assert cookie_path.exists(), f"Cookie file not found at {cookie_path}"
    print(f"Cookie file found at {cookie_path}")

    # Generate blocks
    subprocess.run(
        [
            "bitcoin-cli",
            "-regtest",
            f"-datadir={bitcoin_dir}",
            f"-rpcport={rpcport}",
            "createwallet",
            "test",
        ],
        capture_output=True,
    )
    subprocess.run(
        [
            "bitcoin-cli",
            "-regtest",
            f"-datadir={bitcoin_dir}",
            f"-rpcport={rpcport}",
            "-generate",
            "101",
        ],
        capture_output=True,
    )

    # Start lightningd with smaug using cookie_dir
    print(f"Starting lightningd with smaug in {lightning_dir}")
    subprocess.Popen(
        [
            "lightningd",
            "--network=regtest",
            f"--lightning-dir={lightning_dir}",
            f"--bitcoin-datadir={bitcoin_dir}",
            f"--bitcoin-rpcport={rpcport}",
            f"--addr=0.0.0.0:{ln_port}",
            "--disable-plugin=cln-grpc",
            f"--plugin={get_plugin()}",
            f"--smaug_brpc_cookie_dir={bitcoin_dir}/regtest",
            f"--smaug_brpc_port={rpcport}",
            "--daemon",
            f"--log-file={log_file}",
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    if not wait_for_lightningd(lightning_dir):
        # Print log file for debugging
        if log_file.exists():
            print(f"=== lightningd.log ===\n{log_file.read_text()}\n===")
        assert False, "lightningd failed to start"

    result = subprocess.run(
        [
            "lightning-cli",
            f"--lightning-dir={lightning_dir}",
            "--network=regtest",
            "smaug",
            "ls",
        ],
        capture_output=True,
        text=True,
    )

    print(f"smaug ls output: {result.stdout}")
    assert result.returncode == 0, f"smaug ls failed: {result.stderr}"
    assert (
        "not_configured" not in result.stdout
    ), "smaug reports not configured"
    print("SUCCESS: Cookie file authentication works!")


@pytest.mark.manual
def test_user_without_pass_fails(manual_test_dirs):
    """Test that specifying user without password fails with clear error."""
    bitcoin_dir = manual_test_dirs["bitcoin"]
    lightning_dir = manual_test_dirs["lightning"]
    rpcport = manual_test_dirs["rpcport"]
    p2p_port = manual_test_dirs["p2p_port"]

    print(f"\nStarting bitcoind in {bitcoin_dir} (rpcport={rpcport})")
    subprocess.Popen(
        [
            "bitcoind",
            "-regtest",
            f"-datadir={bitcoin_dir}",
            f"-rpcport={rpcport}",
            f"-port={p2p_port}",
            "-fallbackfee=0.0001",
            "-daemon",
        ],
    )
    assert wait_for_bitcoind(bitcoin_dir, rpcport), "bitcoind failed to start"

    subprocess.run(
        [
            "bitcoin-cli",
            "-regtest",
            f"-datadir={bitcoin_dir}",
            f"-rpcport={rpcport}",
            "createwallet",
            "test",
        ],
        capture_output=True,
    )
    subprocess.run(
        [
            "bitcoin-cli",
            "-regtest",
            f"-datadir={bitcoin_dir}",
            f"-rpcport={rpcport}",
            "-generate",
            "101",
        ],
        capture_output=True,
    )

    print("Starting lightningd with smaug_brpc_user but no password...")
    try:
        result = subprocess.run(
            [
                "lightningd",
                "--network=regtest",
                f"--lightning-dir={lightning_dir}",
                f"--bitcoin-datadir={bitcoin_dir}",
                f"--bitcoin-rpcport={rpcport}",
                f"--plugin={get_plugin()}",
                "--smaug_brpc_user=testuser",
            ],
            capture_output=True,
            text=True,
            timeout=15,
        )
        output = result.stdout + result.stderr
    except subprocess.TimeoutExpired as e:
        output = ""
        if e.stdout:
            output += e.stdout.decode()
        if e.stderr:
            output += e.stderr.decode()

    print(f"lightningd output: {output}")
    assert (
        "smaug_brpc_pass" in output
    ), f"Expected error about missing smaug_brpc_pass, got: {output}"
    print("SUCCESS: User without password fails with clear error!")


@pytest.mark.manual
def test_invalid_cookie_dir_fails(manual_test_dirs):
    """Test that specifying nonexistent cookie dir fails with clear error."""
    bitcoin_dir = manual_test_dirs["bitcoin"]
    lightning_dir = manual_test_dirs["lightning"]
    rpcport = manual_test_dirs["rpcport"]
    p2p_port = manual_test_dirs["p2p_port"]

    print(f"\nStarting bitcoind in {bitcoin_dir} (rpcport={rpcport})")
    subprocess.Popen(
        [
            "bitcoind",
            "-regtest",
            f"-datadir={bitcoin_dir}",
            f"-rpcport={rpcport}",
            f"-port={p2p_port}",
            "-fallbackfee=0.0001",
            "-daemon",
        ],
    )
    assert wait_for_bitcoind(bitcoin_dir, rpcport), "bitcoind failed to start"

    subprocess.run(
        [
            "bitcoin-cli",
            "-regtest",
            f"-datadir={bitcoin_dir}",
            f"-rpcport={rpcport}",
            "createwallet",
            "test",
        ],
        capture_output=True,
    )
    subprocess.run(
        [
            "bitcoin-cli",
            "-regtest",
            f"-datadir={bitcoin_dir}",
            f"-rpcport={rpcport}",
            "-generate",
            "101",
        ],
        capture_output=True,
    )

    print("Starting lightningd with invalid smaug_brpc_cookie_dir...")
    try:
        result = subprocess.run(
            [
                "lightningd",
                "--network=regtest",
                f"--lightning-dir={lightning_dir}",
                f"--bitcoin-datadir={bitcoin_dir}",
                f"--bitcoin-rpcport={rpcport}",
                f"--plugin={get_plugin()}",
                "--smaug_brpc_cookie_dir=/nonexistent/path",
            ],
            capture_output=True,
            text=True,
            timeout=15,
        )
        output = result.stdout + result.stderr
    except subprocess.TimeoutExpired as e:
        output = ""
        if e.stdout:
            output += e.stdout.decode()
        if e.stderr:
            output += e.stderr.decode()

    print(f"lightningd output: {output}")
    assert (
        "nonexistent" in output.lower() or "cookie" in output.lower()
    ), f"Expected error about nonexistent cookie file, got: {output}"
    print("SUCCESS: Invalid cookie dir fails with clear error!")


@pytest.mark.manual
def test_standard_cookie_path_detection(manual_test_dirs):
    """Priority 4: Auto-detect cookie at standard ~/.bitcoin path.

    Creates a temporary .bitcoin directory and sets HOME to point to it.
    """
    lightning_dir = manual_test_dirs["lightning"]
    rpcport = manual_test_dirs["rpcport"]
    p2p_port = manual_test_dirs["p2p_port"]
    ln_port = manual_test_dirs["ln_port"]
    log_file = Path(lightning_dir) / "lightningd.log"

    fake_home = tempfile.mkdtemp(prefix="smaug-test-home-")
    fake_bitcoin_dir = Path(fake_home) / ".bitcoin"
    fake_bitcoin_dir.mkdir()

    try:
        (fake_bitcoin_dir / "bitcoin.conf").write_text("")

        print(f"\nStarting bitcoind in fake home: {fake_bitcoin_dir}")
        subprocess.Popen(
            [
                "bitcoind",
                "-regtest",
                f"-datadir={fake_bitcoin_dir}",
                f"-rpcport={rpcport}",
                f"-port={p2p_port}",
                "-fallbackfee=0.0001",
                "-daemon",
            ],
        )

        time.sleep(3)
        cookie_path = fake_bitcoin_dir / "regtest" / ".cookie"

        if not cookie_path.exists():
            pytest.skip("bitcoind didn't create cookie file")

        print(f"Cookie file created at {cookie_path}")

        subprocess.run(
            [
                "bitcoin-cli",
                "-regtest",
                f"-datadir={fake_bitcoin_dir}",
                f"-rpcport={rpcport}",
                "createwallet",
                "test",
            ],
            capture_output=True,
        )
        subprocess.run(
            [
                "bitcoin-cli",
                "-regtest",
                f"-datadir={fake_bitcoin_dir}",
                f"-rpcport={rpcport}",
                "-generate",
                "101",
            ],
            capture_output=True,
        )

        env = os.environ.copy()
        env["HOME"] = fake_home

        print(f"Starting lightningd with HOME={fake_home}")
        subprocess.Popen(
            [
                "lightningd",
                "--network=regtest",
                f"--lightning-dir={lightning_dir}",
                f"--bitcoin-datadir={fake_bitcoin_dir}",
                f"--bitcoin-rpcport={rpcport}",
                f"--addr=0.0.0.0:{ln_port}",
                "--disable-plugin=cln-grpc",
                f"--plugin={get_plugin()}",
                f"--smaug_brpc_port={rpcport}",
                "--daemon",
                f"--log-file={log_file}",
            ],
            env=env,
        )

        if not wait_for_lightningd(lightning_dir):
            # Print log file for debugging
            if log_file.exists():
                print(f"=== lightningd.log ===\n{log_file.read_text()}\n===")
            assert False, "lightningd failed to start"

        result = subprocess.run(
            [
                "lightning-cli",
                f"--lightning-dir={lightning_dir}",
                "--network=regtest",
                "smaug",
                "ls",
            ],
            capture_output=True,
            text=True,
        )

        print(f"smaug ls output: {result.stdout}")
        assert result.returncode == 0, f"smaug ls failed: {result.stderr}"
        assert (
            "not_configured" not in result.stdout
        ), "smaug reports not configured"
        print("SUCCESS: Standard cookie path detection works!")

    finally:
        stop_lightningd(lightning_dir)
        stop_bitcoind(str(fake_bitcoin_dir), rpcport)
        time.sleep(1)
        shutil.rmtree(fake_home, ignore_errors=True)
