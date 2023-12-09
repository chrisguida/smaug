# tests for smaug
# does not work with nix develop yet. exit out of devShell before executing tests
# run
# cd tests
# poetry shell
# poetry install
# poetry run pytest test.py --log-cli-level=INFO -s

import pytest
from pyln.client import RpcError
from conftest import SMAUG_PLUGIN

@pytest.mark.developer("Requires dev_sign_last_tx")
def test_smaug(node_factory, bitcoind):
    """
    Test Smaug.

    """

    l1, l2 = node_factory.line_graph(2, opts=[{"allow_broken_log": True}, {"plugin": SMAUG_PLUGIN}])

    assert 1==1
