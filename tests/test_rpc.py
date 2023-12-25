from pprint import pprint

from conftest import SMAUG_PLUGIN
from fixtures import *
from pyln.client import Millisatoshi
from pyln.testing.utils import BITCOIND_CONFIG, only_one, wait_for
from utils import *
import os


def test_rpc_remove(bitcoind, ln_node):
    """
    Test RPC remove.
    """

    # get external/internal only_one descriptors
    internal_descriptor = get_only_one_descriptor(bitcoind, "wpkh", True)
    external_descriptor = get_only_one_descriptor(bitcoind, "wpkh", False)

    # add wallet to smaug
    wallet = ln_node.rpc.smaug("add", external_descriptor, internal_descriptor)
    wallet_name = wallet["name"]
    db_file_path = f"{str(ln_node.lightning_dir)}/regtest/.smaug/{wallet_name}.db"

    smaug_wallets = ln_node.rpc.smaug("ls")
    assert len(smaug_wallets) == 1
    assert wallet_name in smaug_wallets
    assert os.path.isfile(db_file_path)

    # remove wallet from smaug
    result = ln_node.rpc.smaug("remove", wallet_name)

    smaug_wallets = ln_node.rpc.smaug("ls")
    assert len(smaug_wallets) == 0
    assert result == f"Deleted wallet: {wallet_name}"
    assert not os.path.isfile(db_file_path)


def test_rpc_list(bitcoind, ln_node):
    """
    Test RPC list.
    """

    # get external/internal only_one descriptors
    external_descriptor_wpkh = get_only_one_descriptor(bitcoind, "wpkh", False)
    internal_descriptor_wpkh = get_only_one_descriptor(bitcoind, "wpkh", True)
    external_descriptor_tr = get_only_one_descriptor(bitcoind, "tr", False)
    internal_descriptor_tr = get_only_one_descriptor(bitcoind, "tr", True)

    # add two wallets to smaug
    wallet1 = ln_node.rpc.smaug("add", external_descriptor_wpkh, internal_descriptor_wpkh, "821000", "5000")
    wallet2 = ln_node.rpc.smaug("add", external_descriptor_tr, internal_descriptor_tr, "821001", "5001")
    wallet1_name = wallet1["name"]
    wallet2_name = wallet2["name"]

    # list smaug wallets
    smaug_wallets = ln_node.rpc.smaug("ls")

    assert len(smaug_wallets) == 2
    assert wallet1_name in smaug_wallets
    assert wallet2_name in smaug_wallets

    smaug_wallet_1 = smaug_wallets[wallet1_name]
    smaug_wallet_2 = smaug_wallets[wallet2_name]

    assert smaug_wallet_1["change_descriptor"].startswith("wpkh(")
    assert smaug_wallet_2["change_descriptor"].startswith("tr(")

    assert smaug_wallet_1["descriptor"].startswith("wpkh(")
    assert smaug_wallet_2["descriptor"].startswith("tr(")

    assert smaug_wallet_1["birthday"] == 821000
    assert smaug_wallet_2["birthday"] == 821001

    assert smaug_wallet_1["gap"] == 5000
    assert smaug_wallet_2["gap"] == 5001
