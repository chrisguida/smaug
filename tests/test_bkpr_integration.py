# tests for smaug
#
# does not work with nix develop yet.
# exit out of devShell before executing tests.
#
# running the tests:
# cd tests
# poetry shell
# poetry install
# poetry run pytest test.py --log-cli-level=INFO -s

from decimal import Decimal
from pprint import pprint

from pyln.client import Millisatoshi
from pyln.testing.utils import only_one, wait_for
from utils import (
    btc_to_sats,
    generate_to_mining_wallet,
    get_bitcoind_wallet_bal_sats,
    get_cln_balance,
    get_descriptor,
    sats_to_btc,
    send_from_wallet,
    switch_wallet,
)


def test_bkpr_integration(ln_node, bitcoind):
    """
    Test Smaug and bkpr integration
    Note: this test is invalid until bkpr shared tx income calculation is fixed
    """

    def generate():
        generate_to_mining_wallet(
            bitcoind, MINING_WALLET_NAME, SMAUG_WALLET_NAME
        )

    MINING_WALLET_NAME = "lightningd-tests"
    SMAUG_WALLET_NAME = "smaug_test"
    SMAUG_INITIAL_AMOUNT_SAT = 100_000_000
    SMAUG_INITIAL_AMOUNT_BTC = sats_to_btc(SMAUG_INITIAL_AMOUNT_SAT)
    CLN_INITIAL_AMOUNT_SAT = 10_000_000
    CLN_INITIAL_AMOUNT_MSAT = Millisatoshi(CLN_INITIAL_AMOUNT_SAT * 10**3)

    # make a new wallet to use with smaug because the default wallet is full
    # of coinbases, which muddles the test results.
    bitcoind.rpc.createwallet(SMAUG_WALLET_NAME)
    switch_wallet(bitcoind, SMAUG_WALLET_NAME)

    # ## simple receive ## #
    # n inputs (we ignore all of these because none are ours)
    # n outputs, 1 of which is ours (send utxo_deposit from external account
    # to our account)

    # fund our smaug wallet with 1BTC (100_000_000sat)
    # this should create our first utxo_deposit event
    # (once we add the wallet to smaug)
    initial_smaug_receive_addr = bitcoind.rpc.getnewaddress()
    send_from_wallet(
        bitcoind,
        MINING_WALLET_NAME,
        initial_smaug_receive_addr,
        SMAUG_INITIAL_AMOUNT_BTC,
    )
    generate()
    assert get_bitcoind_wallet_bal_sats(bitcoind) == btc_to_sats(
        SMAUG_INITIAL_AMOUNT_BTC
    )

    # ## simple spend ## #
    # 1 input which is ours (send utxo_spend for 100M sats)
    # 2 outputs:
    #   1 which is the 10M sat spend (to an external account)
    #   (send utxo_deposit from our account to external)
    #   1 which is 89_999_859 sat change (back to our wallet)
    #   (send utxo_deposit from our account back to our account)
    # fee on regtest is currently 141 sats, not sure if this will change in
    # the future

    # to do our spend, we send 10M sats to the CLN internal wallet from our
    # smaug wallet
    # this subtracts 1M sat (+141 sats for fee) from our bitcoind wallet
    # this will generate 3 more bkpr events for our smaug wallet:
    # 1 utxo_spend for our input and 2 utxo_deposits for the outputs
    cln_addr = ln_node.rpc.newaddr(addresstype="bech32")["bech32"]
    bitcoind.rpc.sendtoaddress(cln_addr, sats_to_btc(CLN_INITIAL_AMOUNT_SAT))
    generate()
    # now we should have 100_000_000 - 10_000_141 sats
    assert get_bitcoind_wallet_bal_sats(bitcoind) == 89_999_859

    # wait for funds to show up in CLN
    wait_for(lambda: len(ln_node.rpc.listfunds()["outputs"]) == 1)

    # verify that the 10Msat showed up in the main CLN wallet in bkpr
    bkpr_balances = ln_node.rpc.bkpr_listbalances()["accounts"]
    cln_balance = get_cln_balance(bkpr_balances)
    assert cln_balance["coin_type"] == "bcrt"
    assert cln_balance["balance_msat"] == CLN_INITIAL_AMOUNT_MSAT

    # ## simple shared tx ## #
    # payjoin where we pay 10M sats from smaug (ours) to CLN (theirs)
    # 2 inputs
    #   1 which is ours (89_999_859 sats) (send utxo_spend)
    #   1 which is theirs (10M sats) (send utxo_spend)
    # 2 outputs
    #   1 which is ours (79_998_859 sats)
    #   (utxo_deposit from our wallet to our wallet (basically change))
    #   1 which is theirs (20M sats)
    #   (utxo_deposit from our wallet to their wallet)

    # this will generate 4 more events, one for each input and output
    # first grab our 89_999_859 sat output from above
    unspent = bitcoind.rpc.listunspent()
    pprint(unspent)
    smaug_utxo = only_one(unspent)
    smaug_txout_amount_sat = (
        SMAUG_INITIAL_AMOUNT_SAT - CLN_INITIAL_AMOUNT_SAT - 141
    )
    smaug_txout_amount_msat = Millisatoshi(str(smaug_txout_amount_sat) + "sat")
    assert (
        Millisatoshi(str(smaug_utxo["amount"]) + "btc")
        == smaug_txout_amount_msat
    )  # 89_999_859sat

    # then grab CLN's 10_000_000 sat output
    cln_utxo = only_one(ln_node.rpc.listfunds()["outputs"])
    assert cln_utxo["amount_msat"] == CLN_INITIAL_AMOUNT_MSAT
    cln_addr = ln_node.rpc.newaddr(addresstype="bech32")["bech32"]
    btc_addr = bitcoind.rpc.getnewaddress()

    # total inputs = 10_000_000 + 89_999_859 = 99_999_859
    fee_amt_msat = Millisatoshi("1000sat")

    # total outputs = 99_999_859 - 1000 = 99_998_859

    # amount to CLN = 10_000_000 from CLN plus 10_000_000 from smaug
    # = 20_000_000
    amt_from_smaug_to_cln_btc = Decimal("0.1")
    amt_to_cln_btc = (
        CLN_INITIAL_AMOUNT_MSAT.to_btc() + amt_from_smaug_to_cln_btc
    )

    # amount to Smaug = 99_998_859 - 20_000_000 = 79_998_859
    amt_to_smaug_btc = (
        smaug_txout_amount_msat.to_btc()
        - amt_from_smaug_to_cln_btc
        - fee_amt_msat.to_btc()
    )

    raw_psbt = bitcoind.rpc.createpsbt(
        [
            {"txid": smaug_utxo["txid"], "vout": smaug_utxo["vout"]},
            {"txid": cln_utxo["txid"], "vout": cln_utxo["output"]},
        ],
        [{cln_addr: str(amt_to_cln_btc)}, {btc_addr: str(amt_to_smaug_btc)}],
    )
    assert only_one(ln_node.rpc.reserveinputs(raw_psbt)["reservations"])[
        "reserved"
    ]
    l1_signed_psbt = ln_node.rpc.signpsbt(raw_psbt, [1])["signed_psbt"]
    process_res = bitcoind.rpc.walletprocesspsbt(l1_signed_psbt)
    assert process_res["complete"]

    ln_node.rpc.sendpsbt(process_res["psbt"])["txid"]  # returns txid

    generate()

    # ## add wallet to smaug ## #
    # first get external/internal only_one(descriptors)
    all_descriptors = bitcoind.rpc.listdescriptors()["descriptors"]
    pprint(all_descriptors)
    wpkh_descriptors = list(
        filter(lambda x: x["desc"].startswith("wpkh"), all_descriptors)
    )
    internal_descriptor = get_descriptor(wpkh_descriptors, True)
    external_descriptor = get_descriptor(wpkh_descriptors, False)
    print("internal_descriptor = %s " % internal_descriptor)
    print("external_descriptor = %s " % external_descriptor)

    # print("smaug ls result = %s" % ln_node.rpc.smaug("ls"))
    # name = ln_node.rpc.smaug(
    #     "add",
    #     external_descriptor,
    #     internal_descriptor
    # )["name"]

    # bkpr_balances = ln_node.rpc.bkpr_listbalances()["accounts"]
    # bitcoind_smaug_balance = get_bkpr_smaug_balance(name, bkpr_balances)
    # assert bitcoind_smaug_balance["coin_type"] == "bcrt"
    # assert (
    #     bitcoind_smaug_balance["balance_msat"]
    #     == get_bitcoind_wallet_bal_sats(bitcoind) * 10**3
    # )

    # there should now be 12 events in our bkpr_listaccountevents() call:

    all_accountevents = ln_node.rpc.bkpr_listaccountevents()["events"]

    # 4 for the `wallet` account:
    #   1 journal_entry for 0 msats (this entry will be removed from bkpr
    #     soon?)
    #   2 deposits:
    #     10M sats for initial deposit
    #     20M sats for payjoin output
    #   1 withdrawal:
    #     10M sats for payjoin input

    # smaug_accountevents = list(
    #    filter(lambda x: x["account"] == "smaug:%s" % name, all_accountevents)
    # )
    list(
        filter(lambda x: x["account"] == "external", all_accountevents)
    )  # returns external_accountevents
    list(
        filter(lambda x: x["account"] != "wallet", all_accountevents)
    )  # returns non_cln_accountevents
    list(
        filter(
            lambda x: x.get("outpoint")
            == "42d4c98bbbdd16c11d150e2f9e038466c52341a13adfd81d33d8385a2ef90113:0",  # noqa: E501
            all_accountevents,
        )
    )  # returns accountevents

    # assert len(smaug_accountevents) == 8

    # 8 for the `smaug:{name}` account:
    #   5 deposits:
    #     3 deposits from external into smaug:
    #       100M sats for initial deposit from mining wallet
    #       89_999_859 sats for change from initial send to CLN wallet
    #       79_998_859 sats for change from payjoin
    #     2 deposits from smaug to external (mirror for 2 CLN wallet deposits
    #       above):
    #       10M sats for initial deposit
    #       20M sats for payjoin output
    #   3 withdrawals:
    #       100M sats for initial deposit from mining wallet
    #       10M sats for initial CLN wallet deposit
    #       20M sats for payjoin output

    pprint(all_accountevents)

    all_incomeevents = ln_node.rpc.bkpr_listincome()["income_events"]
    pprint(all_incomeevents)

    # TODO: fix bkpr upstream payjoin calcs and finish this test
