# P3 效果集：索引模式定位评测

数据不入库，位于 `/workspace/avr-effect-set/`（视频 + manifest.json + NOTICE + transcripts/ + frames/）。

## 转写（真实 ASR）
与仓库 preset `whisper-cpp-cpu` 相同的 whisper.cpp CLI 与参数（`-m -f -l -osrt -of`），CPU 8 线程，模型 ggml-medium（多语言）：

    whisper-cli -t 8 -m ggml-medium.bin -f audio/<id>.wav -l <en|zh> -osrt -of transcripts/raw/<id>
    python3 bench/effect/srt_to_transcript.py transcripts/raw/<id>.srt <lang> ggml-medium.bin <whisper.cpp commit> transcripts/<id>.json

音频：`ffmpeg -i <id>.mp4 -ar 16000 -ac 1 audio/<id>.wav`。`srt_to_transcript.py` 复刻 `timeline::parse_srt`，
`_meta` 中记录模型内容指纹（同 `fingerprint::content_fingerprint` 算法）与 sha256。所有评测固定用这份转写。

## 评测
    python3 bench/effect/eval_index.py --set /workspace/avr-effect-set --questions questions.json [--json out.json]

规则见脚本文件头（分段复刻 `avr-mcp/src/index.rs::sections`；概要 A/B/C + 全文上限；BM25 中文字二元组/英文小写词；
段数 < 6 改报 hit@1；visual 单列）。匹配前做 OpenCC 繁→简（whisper medium 的 zh 输出为繁体）。`dummy_questions.json` 仅用于脚本自测，不是效果集题目。
