import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[1]

def load(name):
    spec = importlib.util.spec_from_file_location(name, TOOLS / name)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module

class CookieTests(unittest.TestCase):
    def test_httponly_and_distinct_paths_survive(self):
        module = load('refresh-douyin-cookies.py')
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'cookies.txt'
            path.write_text('#HttpOnly_.douyin.com\tTRUE\t/\tTRUE\t9999999999\tttwid\tprivate\n.douyin.com\tTRUE\t/video\tFALSE\t9999999999\ta\tpath-specific\n', encoding='utf-8')
            module.merge_document_cookie('a=root;b=second;ttwid=wrong', path)
            rows = module.parse_netscape(path)
            self.assertEqual(rows[('.douyin.com', '/', 'ttwid')][6], 'private')
            self.assertTrue(rows[('.douyin.com', '/', 'ttwid')][0].startswith('#HttpOnly_'))
            self.assertEqual(rows[('.douyin.com', '/video', 'a')][6], 'path-specific')
            self.assertEqual(rows[('.douyin.com', '/', 'a')][6], 'root')
            before = path.read_bytes()
            with self.assertRaises(ValueError):
                module.merge_document_cookie('bad=line\nvalue', path)
            self.assertEqual(path.read_bytes(), before)

    def test_json_import_keeps_values_off_child_command_line(self):
        module = load('refresh-douyin-from-json.py')
        with tempfile.TemporaryDirectory() as tmp:
            source, target = Path(tmp)/'input.json', Path(tmp)/'output.txt'
            source.write_text(json.dumps({'cookie':'a=synthetic','xmst':'synthetic-token'}))
            with patch.object(sys, 'argv', ['import', str(source), '--output', str(target)]), patch.object(subprocess, 'run', side_effect=AssertionError('unexpected subprocess')):
                self.assertEqual(module.main(), 0)
            self.assertIn('synthetic-token', target.read_text())

    def test_installer_preserves_existing_configuration(self):
        module = load('install-to-e-drive.py')
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)/'data'
            root.mkdir()
            original = 'custom = "preserve"\n'
            (root/'config.toml').write_text(original)
            with patch.object(module, 'DATA_ROOT', root), patch.dict(os.environ, {'APPDATA': str(Path(tmp)/'app')}):
                module.write_config()
            self.assertEqual((root/'config.toml').read_text(), original)

if __name__ == '__main__':
    unittest.main()
