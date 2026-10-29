import sys
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'fnos/app'))
from remember import RememberAuth


class RememberTests(unittest.TestCase):
    def setUp(self):
        self.folder=tempfile.TemporaryDirectory();self.addCleanup(self.folder.cleanup)
        self.hashed='synthetic-locked-password-hash'
        self.auth=RememberAuth(Path(self.folder.name)/'auth-secret','hotspot',lookup=lambda user:self.hashed)

    def test_survives_service_restart_without_saving_password(self):
        token=self.auth.issue('test-admin',self.hashed)
        restarted=RememberAuth(self.auth.path,'hotspot',lookup=lambda user:self.hashed)
        self.assertTrue(restarted.authenticate({'remember_token':token})['ok'])
        self.assertNotIn(self.hashed,token)
        self.assertEqual(len(self.auth.path.read_bytes()),32)

    def test_password_change_invalidates_the_old_token(self):
        token=self.auth.issue('test-admin',self.hashed)
        self.hashed='changed-password-hash'
        self.assertFalse(self.auth.authenticate({'remember_token':token})['ok'])

    def test_tampering_expiry_account_and_app_boundaries(self):
        token=self.auth.issue('test-admin',self.hashed)
        self.assertFalse(self.auth.authenticate({'remember_token':token+'x'})['ok'])
        self.assertFalse(self.auth.authenticate({'remember_token':token,'username':'another-admin'})['ok'])
        other=RememberAuth(self.auth.path,'network-panel',lookup=lambda user:self.hashed)
        self.assertFalse(other.authenticate({'remember_token':token})['ok'])
        with patch('remember.time.time',return_value=9999999999):
            self.assertFalse(self.auth.authenticate({'remember_token':token})['ok'])

    def test_removed_administrator_is_not_remembered(self):
        token=self.auth.issue('test-admin',self.hashed)
        self.auth.lookup=lambda _:(_ for _ in ()).throw(ValueError('not an administrator'))
        self.assertFalse(self.auth.authenticate({'remember_token':token})['ok'])


if __name__=='__main__':unittest.main()
