"""只使用临时目录和合成 Cookie，禁止访问真实登录文件。"""
import importlib.util
import os
import sys
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
ROOT = Path(__file__).resolve().parents[1]
def module(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), ROOT / 'tools' / (name + '.py'))
    result = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = result
    spec.loader.exec_module(result)
    return result
class ToolTests(unittest.TestCase):
    def test_httponly_survives_merge(self):
        m = module('refresh-douyin-cookies')
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'synthetic.txt'
            path.write_text('# Netscape HTTP Cookie File\n#HttpOnly_.douyin.com\tTRUE\t/\tTRUE\t2000000000\tttwid\tsynthetic\n')
            m.merge_document_cookie('msToken=fake;ttwid=must-not-replace', path)
            text = path.read_text()
            self.assertIn('#HttpOnly_.douyin.com', text)
            self.assertIn('ttwid\tsynthetic', text)
            self.assertIn('msToken\tfake', text)
    def test_existing_install_config_and_pointer_survive(self):
        m = module('install-to-e-drive')
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            m.DATA_ROOT = root / 'data'; m.MODELS = root / 'models'; m.FFMPEG = root / 'ffmpeg';m.YTDLP = root / 'ytdlp';m.WHISPER = root / 'whisper'
            m.DATA_ROOT.mkdir(); cfg = m.DATA_ROOT / 'config.toml'; cfg.write_text('custom = "preserve"\n')
            appdata=root/'appdata'; pointer=appdata/'AIVideoReader/root.txt';pointer.parent.mkdir(parents=True);pointer.write_text('other-root')
            with patch.dict(os.environ, {'APPDATA': str(appdata)}):m.write_config()
            self.assertEqual(cfg.read_text(),'custom = "preserve"\n');self.assertEqual(pointer.read_text(),'other-root')
    def test_download_fallback_has_cookie_argument(self):
        m=module('douyin-analyze-retry')
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);m.YTDLP=root/'fake-ytdlp';m.YTDLP.touch();m.COOKIES=root/'synthetic-cookies';m.AVR=root/'fake-avr'
            def fake_run(args,**kwargs):
                if args[0]==str(m.YTDLP):
                    self.assertEqual(args[args.index('--cookies')+1],str(m.COOKIES));(root/'download/douyin-fallback.mp4').touch();return subprocess.CompletedProcess(args,0,'','')
                return subprocess.CompletedProcess(args,0,'{"cache_dir":"synthetic-cache"}','')
            with patch.object(m,'run',fake_run):self.assertEqual(m.download_and_transcribe('https://example.invalid',root/'download'),Path('synthetic-cache'))
if __name__=='__main__':unittest.main()
