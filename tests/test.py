# tests for smaug
# does not work with nix develop yet. exit out of devShell before executing tests
# run
# cd tests
# poetry shell
# poetry install
# poetry run pytest test.py --log-cli-level=INFO -s

from pprint import pprint
import time
import pytest
from pyln.client import RpcError
from conftest import SMAUG_PLUGIN
from pyln.testing.utils import BITCOIND_CONFIG

# @pytest.mark.developer("Requires dev_sign_last_tx")
def test_smaug(node_factory, bitcoind):
    """
    Test Smaug.

    """

    l1 = node_factory.get_nodes(1, opts={"allow_broken_log": True, "plugin": SMAUG_PLUGIN, "smaug_brpc_user": BITCOIND_CONFIG['rpcuser'], "smaug_brpc_pass": BITCOIND_CONFIG['rpcpassword']})

    info = bitcoind.rpc.getblockchaininfo()
    pprint(info)

    # time.sleep(10)

    assert 1==1
