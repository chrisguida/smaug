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
from utils import *


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
    assert get_bitcoind_wallet_bal_sats(bitcoind) == btc_to_sats(5050)

    cln_addr = l1.rpc.newaddr()["bech32"]

    cln_initial_amount = 1000000
    cln_initial_amount_msat = Millisatoshi(cln_initial_amount * 10**3)
    # this subtracts 1M sat from our bitcoind wallet
    bitcoind.rpc.sendtoaddress(cln_addr, sats_to_btc(cln_initial_amount))
    # this adds 50 btc to our bitcoind wallet
    bitcoind.generate_block(1)
    assert get_bitcoind_wallet_bal_sats(bitcoind) == 509999000000

    # wait for funds to show up in CLN
    wait_for(lambda: len(l1.rpc.listfunds()["outputs"]) == 1)

    bkpr_balances = l1.rpc.bkpr_listbalances()

    # verify pre-test CLN funds in bkpr
    btc_balance = only_one(only_one(bkpr_balances["accounts"])["balances"])
    assert btc_balance["balance_msat"] == cln_initial_amount_msat

    # get external/internal only_one(descriptors)
    all_descriptors = bitcoind.rpc.listdescriptors()["descriptors"]
    pprint(all_descriptors)
    wpkh_descriptors = list(
        filter(lambda x: x["desc"].startswith("wpkh"), all_descriptors)
    )
    internal_descriptor = get_descriptor(wpkh_descriptors, True)
    external_descriptor = get_descriptor(wpkh_descriptors, False)
    print("internal_descriptor = %s " % internal_descriptor)
    print("external_descriptor = %s " % external_descriptor)

    # add wallet to smaug
    print("smaug ls result = %s" % l1.rpc.smaug("ls"))
    name = l1.rpc.smaug("add", external_descriptor, internal_descriptor)["name"]

    # verify initial funds in wallet
    bkpr_balances = l1.rpc.bkpr_listbalances()["accounts"]

    # verify pre-test CLN funds in bkpr
    cln_balance = get_cln_balance(bkpr_balances)
    assert cln_balance["coin_type"] == "bcrt"
    assert cln_balance["balance_msat"] == cln_initial_amount_msat

    bitcoind_smaug_balance = get_bkpr_smaug_balance(name, bkpr_balances)
    assert bitcoind_smaug_balance["coin_type"] == "bcrt"
    assert bitcoind_smaug_balance["balance_msat"] == get_bitcoind_wallet_bal_sats(bitcoind) * 10**3

    ### already done
    # generate second address
    addr2 = l1.rpc.newaddr()["bech32"]

    # send funds to descriptor wallet address (received)
    ### already done

    # simple spend
    # 1 input which is ours (send utxo_spent)
    # 2 outputs:
    #   1 which is the spend (to an external account) (send utxo_deposit from our account to external)
    #   1 which is change (back to our wallet) (send utxo_deposit from our account back to our account)
    unspent = bitcoind.rpc.listunspent()
    txout = [i for i in unspent if i['amount'] == 50 and i['spendable']][0]
    txout2 = [i for i in l1.rpc.listfunds()['outputs'] if i['status'] == 'confirmed' and not i['reserved']][0]
    assert txout is not None
    assert txout2 is not None
    assert txout2['amount_msat'] == cln_initial_amount_msat
    cln_addr = l1.rpc.newaddr()['bech32']
    btc_addr = bitcoind.rpc.getnewaddress()
    fee_amt = 1000
    amt_btc_wallet = (cln_initial_amount - fee_amt) / 10 ** 8 + 48

    raw_psbt = bitcoind.rpc.createpsbt([{"txid": txout["txid"], "vout": txout["vout"]}, {"txid": txout2["txid"], "vout": txout2["output"]}], [{cln_addr: "2"}, {btc_addr: amt_btc_wallet}])
    assert only_one(l1.rpc.reserveinputs(raw_psbt)['reservations'])['reserved']
    l1_signed_psbt = l1.rpc.signpsbt(raw_psbt, [1])['signed_psbt']
    process_res = bitcoind.rpc.walletprocesspsbt(l1_signed_psbt)
    assert process_res['complete']

    txid = l1.rpc.sendpsbt(process_res['psbt'])['txid']

    bitcoind.generate_block(1)

    assert False
    # simple receive
    # n inputs (we ignore all of these because none are ours)
    # n outputs, 1 of which is ours (send utxo_deposit from external account to our account)

    # simple payjoin where we pay 1BTC
    # 2 inputs
    #   1 which is ours (2BTC) (send utxo_spent)
    #   1 which is theirs (1BTC) (ignore)
    # 2 outputs
    #   1 which is ours (1BTC) (utxo_deposit from our wallet to our wallet (basically change))
    #   1 which is theirs (2BTC) (utxo_deposit from our wallet to their wallet. problem is that we now have 3BTC worth of spends from our wallet in this tx but only 2BTC of inputs)

    # create psbt from smaug wallet


    cln_second_amount = 1234567
    cln_second_amount_msat = Millisatoshi(cln_second_amount * 10**3)
    bitcoind.rpc.sendtoaddress(addr2, sats_to_btc(cln_second_amount))
    bitcoind.generate_block(1)
    assert get_bitcoind_wallet_bal_sats(bitcoind) == 514997765433

    # wait for new funds to show up in CLN
    wait_for(lambda: len(l1.rpc.listfunds()["outputs"]) == 2)

    bkpr_balances = l1.rpc.bkpr_listbalances()["accounts"]

    # verify CLN funds in bkpr
    cln_balance = get_cln_balance(bkpr_balances)
    assert (
        cln_balance["balance_msat"] == cln_initial_amount_msat + cln_second_amount_msat
    )

    wait_for(
        lambda: get_bkpr_smaug_balance(name, bkpr_balances)["balance_msat"]
        == get_bitcoind_wallet_bal_sats(bitcoind) * 10**3
    )

    assert (
        get_bkpr_smaug_balance(name, bkpr_balances)["balance_msat"]
        == get_bitcoind_wallet_bal_sats(bitcoind) * 10**3
    )

    print("Done syncronizing smaug.")
    # catch bkpr log
    # wait_for_log
    # l1.daemon.wait_for_log(r"utxo_deposit \(deposit|nifty's secret stash\) .* -0msat 1679955976 111 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb:0")

    # find event in bkpr events
    events = l1.rpc.bkpr_listaccountevents()["events"]

    # pprint(events)
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
