from pprint import pprint

from conftest import SMAUG_PLUGIN
from fixtures import *
from pyln.client import Millisatoshi
from pyln.testing.utils import BITCOIND_CONFIG, only_one, wait_for
from utils import *
import os
def test_rpc_remove(node_factory, bitcoind):
    """
    Test RPC remove.
    """

    l1 = node_factory.get_nodes(
        1,
        opts={
            "allow_broken_log": True,
            "plugin": SMAUG_PLUGIN,
            "smaug_brpc_user": BITCOIND_CONFIG["rpcuser"],
            "smaug_brpc_pass": BITCOIND_CONFIG["rpcpassword"],
            "smaug_brpc_port": BITCOIND_CONFIG["rpcport"],
        },
    )[0]

    # get external/internal only_one descriptors
    internal_descriptor = get_only_one_descriptor(bitcoind, "wpkh", True)
    external_descriptor = get_only_one_descriptor(bitcoind, "wpkh", False)

    # add wallet to smaug
    wallet = l1.rpc.smaug("add", external_descriptor, internal_descriptor)
    wallet_name = wallet["name"]
    db_file_path = f"{str(l1.lightning_dir)}/regtest/.smaug/{wallet_name}.db"
    smaug_wallets = l1.rpc.smaug("ls")
    assert len(smaug_wallets) == 1
    assert wallet_name in smaug_wallets
    assert os.path.isfile(db_file_path)

    # remove wallet from smaug
    result = l1.rpc.smaug("remove", wallet_name)

    smaug_wallets = l1.rpc.smaug("ls")
    assert len(smaug_wallets) == 0
    assert result == f"Deleted wallet: {wallet_name}"
    assert not os.path.isfile(db_file_path)


def test_rpc_list(node_factory, bitcoind):
    """
    Test RPC list.
    """

    l1 = node_factory.get_nodes(
        1,
        opts={
            "allow_broken_log": True,
            "plugin": SMAUG_PLUGIN,
            "smaug_brpc_user": BITCOIND_CONFIG["rpcuser"],
            "smaug_brpc_pass": BITCOIND_CONFIG["rpcpassword"],
            "smaug_brpc_port": BITCOIND_CONFIG["rpcport"],
        },
    )[0]

    # get external/internal only_one descriptors
    external_descriptor = get_only_one_descriptor(bitcoind, "wpkh", False)
    internal_descriptor = get_only_one_descriptor(bitcoind, "wpkh", True)

    external_descriptor_tr = get_only_one_descriptor(bitcoind, "tr", False)
    internal_descriptor_tr = get_only_one_descriptor(bitcoind, "tr", True)

    # add two wallets to smaug
    wallet1 = l1.rpc.smaug("add", external_descriptor, internal_descriptor)
    wallet2 = l1.rpc.smaug("add", external_descriptor_tr, internal_descriptor_tr)
    wallet1_name = wallet1["name"]
    wallet2_name = wallet2["name"]

    smaug_wallets = l1.rpc.smaug("ls")
    assert len(smaug_wallets) == 2
    assert wallet1_name in smaug_wallets
    assert wallet2_name in smaug_wallets

    wallet1_cd = smaug_wallets[wallet1_name]["change_descriptor"]
    wallet2_cd = smaug_wallets[wallet2_name]["change_descriptor"]
    assert wallet1_cd.startswith("wpkh(")
    assert wallet2_cd.startswith("tr(")

    wallet1_d = smaug_wallets[wallet1_name]["descriptor"]
    wallet2_d = smaug_wallets[wallet2_name]["descriptor"]
    assert wallet1_d.startswith("wpkh(")
    assert wallet2_d.startswith("tr(")
