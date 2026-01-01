import os
import re

import pytest
from pyln.client.lightning import RpcError
from utils import get_bkpr_smaug_balance, get_only_one_descriptor


def test_rpc_add(bitcoind, ln_node):
    """
    Test RPC add
    """

    # Get external/internal only_one descriptors
    external_descriptor_wpkh = get_only_one_descriptor(bitcoind, "wpkh", False)
    internal_descriptor_wpkh = get_only_one_descriptor(bitcoind, "wpkh", True)

    smaug_wallets = ln_node.rpc.smaug("ls")
    assert len(smaug_wallets) == 0

    # Add a wallet to smaug
    wallet = ln_node.rpc.smaug(
        "add",
        external_descriptor_wpkh,
        internal_descriptor_wpkh,
        "821000",
        "5000",
    )
    wallet_name = wallet["name"]

    asserted = {
        "message": f"Wallet with deterministic name {wallet_name} successfully added",  # noqa: E501
        "name": wallet_name,
    }

    assert len(wallet.keys()) == len(asserted.keys())
    for key, value in wallet.items():
        assert value == asserted[key]

    ln_node.daemon.wait_for_log(
        f"Wallet with deterministic name {wallet_name} successfully added"
    )

    # List smaug wallets
    smaug_wallets = ln_node.rpc.smaug("ls")

    assert len(smaug_wallets) == 1
    assert wallet_name in smaug_wallets

    smaug_wallet = smaug_wallets[wallet_name]

    assert smaug_wallet["change_descriptor"].startswith("wpkh(")
    assert smaug_wallet["descriptor"].startswith("wpkh(")
    assert smaug_wallet["birthday"] == 821000
    assert smaug_wallet["gap"] == 5000


def test_rpc_list(bitcoind, ln_node):
    """
    Test RPC list
    """

    # Get external/internal only_one descriptors
    external_descriptor_wpkh = get_only_one_descriptor(bitcoind, "wpkh", False)
    internal_descriptor_wpkh = get_only_one_descriptor(bitcoind, "wpkh", True)
    external_descriptor_tr = get_only_one_descriptor(bitcoind, "tr", False)
    internal_descriptor_tr = get_only_one_descriptor(bitcoind, "tr", True)

    # Add two wallets to smaug
    wallet1 = ln_node.rpc.smaug(
        "add",
        external_descriptor_wpkh,
        internal_descriptor_wpkh,
        "821000",
        "5000",
    )
    wallet2 = ln_node.rpc.smaug(
        "add", external_descriptor_tr, internal_descriptor_tr, "821001", "5001"
    )
    wallet1_name = wallet1["name"]
    wallet2_name = wallet2["name"]

    # List smaug wallets
    smaug_wallets = ln_node.rpc.smaug("ls")

    assert len(smaug_wallets) == 2
    assert wallet1_name in smaug_wallets
    assert wallet2_name in smaug_wallets

    smaug_wallet_1 = smaug_wallets[wallet1_name]
    smaug_wallet_2 = smaug_wallets[wallet2_name]

    asserted_keys = [
        "balance",
        "birthday",
        "change_descriptor",
        "descriptor",
        "gap",
        "network",
    ]
    assert (
        sorted(list(smaug_wallet_1.keys()))
        == sorted(list(smaug_wallet_2.keys()))
        == asserted_keys
    )
    regex_ns_d = r"^wpkh\(\[[a-f0-9]{8}\/84(h|')\/1(h|')\/0(h|')\]tpub[a-zA-Z0-9]{107}\/0\/\*\)#[a-z0-9]{8}$"  # noqa: E501
    regex_ns_cd = r"^wpkh\(\[[a-f0-9]{8}\/84(h|')\/1(h|')\/0(h|')\]tpub[a-zA-Z0-9]{107}\/1\/\*\)#[a-z0-9]{8}$"  # noqa: E501
    regex_tr_d = r"^tr\(\[[a-f0-9]{8}\/86(h|')\/1(h|')\/0(h|')\]tpub[a-zA-Z0-9]{107}\/0\/\*\)#[a-z0-9]{8}$"  # noqa: E501
    regex_tr_cd = r"^tr\(\[[a-f0-9]{8}\/86(h|')\/1(h|')\/0(h|')\]tpub[a-zA-Z0-9]{107}\/1\/\*\)#[a-z0-9]{8}$"  # noqa: E501
    assert re.search(regex_ns_d, smaug_wallet_1["descriptor"]) is not None
    assert (
        re.search(regex_ns_cd, smaug_wallet_1["change_descriptor"]) is not None
    )
    assert smaug_wallet_1["birthday"] == 821000
    assert smaug_wallet_1["gap"] == 5000
    assert re.search(regex_tr_d, smaug_wallet_2["descriptor"]) is not None
    assert (
        re.search(regex_tr_cd, smaug_wallet_2["change_descriptor"]) is not None
    )
    assert smaug_wallet_2["birthday"] == 821001
    assert smaug_wallet_2["gap"] == 5001

    # Verify balances
    asserted_balance = 505000000000
    assert smaug_wallet_1["balance"] == asserted_balance
    assert smaug_wallet_2["balance"] == 0

    # Additionally verify balance against bookkeeper
    bkpr_balances = ln_node.rpc.bkpr_listbalances()["accounts"]
    bitcoind_smaug_balance = get_bkpr_smaug_balance(
        wallet1_name, bkpr_balances
    )
    assert bitcoind_smaug_balance["balance_msat"] / 1000 == asserted_balance


def test_rpc_remove(bitcoind, ln_node):
    """
    Test RPC remove
    """

    # Get external/internal only_one descriptors
    internal_descriptor = get_only_one_descriptor(bitcoind, "wpkh", True)
    external_descriptor = get_only_one_descriptor(bitcoind, "wpkh", False)

    # Add wallet to smaug
    wallet = ln_node.rpc.smaug("add", external_descriptor, internal_descriptor)
    wallet_name = wallet["name"]
    db_file_path = (
        f"{str(ln_node.lightning_dir)}/regtest/.smaug/{wallet_name}.db"
    )

    smaug_wallets = ln_node.rpc.smaug("ls")
    assert len(smaug_wallets) == 1
    assert wallet_name in smaug_wallets
    assert os.path.isfile(db_file_path)

    # Remove wallet from smaug
    result = ln_node.rpc.smaug("remove", wallet_name)

    smaug_wallets = ln_node.rpc.smaug("ls")
    assert len(smaug_wallets) == 0
    assert result == f"Deleted wallet: {wallet_name}"
    assert not os.path.isfile(db_file_path)


def test_rpc_remove_failed(bitcoind, ln_node):
    """
    Test failing RPC remove
    """

    # Get external/internal only_one descriptors
    internal_descriptor = get_only_one_descriptor(bitcoind, "wpkh", True)
    external_descriptor = get_only_one_descriptor(bitcoind, "wpkh", False)

    # Add wallet to smaug
    wallet = ln_node.rpc.smaug("add", external_descriptor, internal_descriptor)
    wallet_name = wallet["name"]
    db_file_path = (
        f"{str(ln_node.lightning_dir)}/regtest/.smaug/{wallet_name}.db"
    )

    smaug_wallets = ln_node.rpc.smaug("ls")
    assert len(smaug_wallets) == 1
    assert wallet_name in smaug_wallets
    assert os.path.isfile(db_file_path)

    # Try removing nonexistent wallet from smaug
    with pytest.raises(
        RpcError,
        match=r'RPC call failed: method: smaug, payload: \(\'remove\', \'NONEXISTENT_WALLET\'\), error: \{\'code\': -32700, \'data\': None, \'message\': "Can\'t find wallet \'NONEXISTENT_WALLET\'."\}',  # noqa: E501
    ):
        ln_node.rpc.smaug("remove", "NONEXISTENT_WALLET")

    smaug_wallets = ln_node.rpc.smaug("ls")
    assert len(smaug_wallets) == 1
    assert os.path.isfile(db_file_path)
