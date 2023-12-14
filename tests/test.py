# tests for smaug
# does not work with nix develop yet. exit out of devShell before executing tests
# run
# cd tests
# poetry shell
# poetry install
# poetry run pytest test.py --log-cli-level=INFO -s

from pprint import pprint

from conftest import SMAUG_PLUGIN
from fixtures import *
from pyln.client import Millisatoshi
from pyln.testing.utils import BITCOIND_CONFIG, only_one, wait_for


# @pytest.mark.developer("Requires dev_sign_last_tx")
def test_smaug(node_factory, bitcoind):
    """
    Test Smaug.
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

    # we start the test with 101 blocks, all of which have coinbases paying to our wallet
    # 100 of these coinbases are not mature, but the earliest one is
    wi_res = bitcoind.rpc.getwalletinfo()
    bitcoind_wallet_bal = wi_res["balance"] + wi_res["immature_balance"]
    pprint(bitcoind.rpc.getwalletinfo())
    assert bitcoind_wallet_bal == 5050

    addr = l1.rpc.newaddr()["bech32"]

    cln_initial_amount = 1000000
    cln_initial_amount_msat = Millisatoshi(cln_initial_amount * 1000)
    # this subtracts 1M sat from our bitcoind wallet
    bitcoind.rpc.sendtoaddress(addr, cln_initial_amount / 10**8)
    # this adds 50 btc to our bitcoind wallet
    bitcoind.generate_block(1)
    wi_res = bitcoind.rpc.getwalletinfo()
    bitcoind_wallet_bal = int(wi_res["balance"] * 10**8) + int(
        wi_res["immature_balance"] * 10**8
    )
    pprint(bitcoind.rpc.getwalletinfo())
    assert bitcoind_wallet_bal == 509999000000

    # wait for funds to show up in CLN
    wait_for(lambda: len(l1.rpc.listfunds()["outputs"]) == 1)

    balances = l1.rpc.bkpr_listbalances()
    pprint(balances)

    # verify pre-test CLN funds in bkpr
    btc_balance = only_one(only_one(balances["accounts"])["balances"])
    assert btc_balance["balance_msat"] == cln_initial_amount_msat

    # get external/nternal only_one(descriptors)
    pprint(bitcoind.rpc.listdescriptors()["descriptors"])
    all_descriptors = bitcoind.rpc.listdescriptors()["descriptors"]
    wpkh_descriptors = list(
        filter(lambda x: x["desc"].startswith("wpkh"), all_descriptors)
    )
    internal_descriptor = only_one(
        list(filter(lambda x: x["internal"] is True, wpkh_descriptors))
    )["desc"]
    external_descriptor = only_one(
        list(filter(lambda x: x["internal"] is False, wpkh_descriptors))
    )["desc"]
    print("internal_descriptor = %s " % internal_descriptor)
    print("external_descriptor = %s " % external_descriptor)

    # add wallet to smaug
    print("smaug ls result = %s" % l1.rpc.smaug("ls"))
    name = l1.rpc.smaug("add", external_descriptor, internal_descriptor)["name"]
    print("name = %s" % name)

    # verify initial funds in wallet
    balances = l1.rpc.bkpr_listbalances()["accounts"]
    pprint(balances)

    # verify pre-test CLN funds in bkpr
    cln_balance = only_one(
        only_one(list(filter(lambda x: x["account"] == "wallet", balances)))["balances"]
    )
    assert cln_balance["coin_type"] == "bcrt"
    # print('smaug:{name}')
    assert cln_balance["balance_msat"] == cln_initial_amount_msat
    bitcoind_smaug_balance = only_one(
        only_one(list(filter(lambda x: x["account"] == "smaug:%s" % name, balances)))[
            "balances"
        ]
    )
    assert bitcoind_smaug_balance["coin_type"] == "bcrt"
    # print(bitcoind_smaug_balance['balance_msat'] + cln_initial_amount_msat)
    # print(int(bitcoind_wallet_bal * 10**3))
    assert bitcoind_smaug_balance["balance_msat"] == int(bitcoind_wallet_bal * 10**3)

    # generate address
    # send funds to descriptor wallet address (received)
    # catch bkpr log
    # find event in bkpr events
    # verify new balance
    # send funds back from smaug wallet to CLN wallet (sent)
    # catch bkpr log
    # find event in bkpr events
    # verify new balance
    # create payjoin between smaug wallet and CLN wallet (shared)
    # catch bkpr log
    # find event in bkpr events
    # verify new balance

    assert 1 == 1
