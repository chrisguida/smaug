from pyln.testing.utils import only_one


def sats_to_btc(sats):
    return sats / 10**8


def btc_to_sats(btc):
    return btc * 10**8


def get_bitcoind_wallet_bal_sats(bitcoind):
    wi_res = bitcoind.rpc.getwalletinfo()
    return int(btc_to_sats(wi_res["balance"] + wi_res["immature_balance"]))


def get_bkpr_smaug_balance(name, bkpr_balances):
    return only_one(
        only_one(
            list(filter(lambda x: x["account"] == "smaug:%s" % name, bkpr_balances))
        )["balances"]
    )


def get_cln_balance(balances):
    return only_one(
        only_one(list(filter(lambda x: x["account"] == "wallet", balances)))["balances"]
    )


def get_descriptor(wpkh_descriptors, internal):
    return only_one(
        list(filter(lambda x: x["internal"] is internal, wpkh_descriptors))
    )["desc"]
