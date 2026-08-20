from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import numpy as np
import soundfile as sf

MODULE_PATH = Path(__file__).with_name("generate.py")
SPEC = importlib.util.spec_from_file_location("sfx_generate", MODULE_PATH)
assert SPEC and SPEC.loader
generate_module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(generate_module)


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.manifest = Path(__file__).with_name("prompts.json")

    def test_manifest_has_exact_coverage_and_unique_values(self):
        entries = generate_module.load_manifest(self.manifest)
        self.assertEqual({entry["name"] for entry in entries}, generate_module.EXPECTED_NAMES)
        self.assertEqual(len({entry["filename"] for entry in entries}), 10)
        self.assertEqual(len({entry["seed"] for entry in entries}), 10)
        self.assertTrue(all(0.05 <= entry["duration"] <= 30 for entry in entries))

    def test_unknown_effect_is_rejected(self):
        entries = generate_module.load_manifest(self.manifest)
        with self.assertRaisesRegex(ValueError, "unknown effect"):
            generate_module.select_entries(entries, ["missing"])

    def test_selection_preserves_requested_order_and_seed(self):
        entries = generate_module.load_manifest(self.manifest)
        selected = generate_module.select_entries(entries, ["menu_confirm", "gun_fire"])
        self.assertEqual([entry["name"] for entry in selected], ["menu_confirm", "gun_fire"])
        self.assertEqual([entry["seed"] for entry in selected], [12009, 12001])


class AudioTests(unittest.TestCase):
    def test_audio_is_stereo_limited_faded_and_exact_length(self):
        raw = np.ones((2, 20_000), dtype=np.float32) * 4
        audio = generate_module.prepare_audio(raw, 0.2)
        self.assertEqual(audio.shape, (8_820, 2))
        self.assertLessEqual(float(np.max(np.abs(audio))), 10 ** (-1 / 20) + 1e-6)
        self.assertTrue(np.all(audio[0] == 0))
        self.assertTrue(np.all(audio[-1] == 0))

    def test_written_fixture_is_44100_stereo_pcm16(self):
        audio = generate_module.prepare_audio(np.ones((2, 1000), dtype=np.float32), 0.01)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.wav"
            sf.write(path, audio, generate_module.SAMPLE_RATE, subtype="PCM_16")
            info = sf.info(path)
        self.assertEqual((info.samplerate, info.channels, info.subtype), (44_100, 2, "PCM_16"))


class WorkflowTests(unittest.TestCase):
    def setUp(self):
        self.entry = {
            "name": "menu_move", "filename": "menu_move.wav", "event": "menu navigation",
            "prompt": "test prompt", "duration": 0.12, "seed": 12008,
        }

    def test_all_skipped_does_not_load_pipeline(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            (output / self.entry["filename"]).touch()
            loader = mock.Mock(side_effect=AssertionError("must not load"))
            written = generate_module.generate([self.entry], output, False, "model", "cpu", 100, 7, loader)
        self.assertEqual(written, [])
        loader.assert_not_called()

    def test_generate_and_force_replace_atomically(self):
        class Result:
            audios = [np.ones((2, 6000), dtype=np.float32)]

        pipeline = mock.Mock(return_value=Result())
        loader = mock.Mock(return_value=(pipeline, "cpu"))
        fake_generator = mock.Mock()
        fake_generator.manual_seed.return_value = fake_generator
        fake_torch = mock.Mock()
        fake_torch.Generator.return_value = fake_generator
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict("sys.modules", {"torch": fake_torch}):
            output = Path(directory)
            written = generate_module.generate([self.entry], output, False, "model", "cpu", 100, 7, loader)
            self.assertEqual(written, [output / "menu_move.wav"])
            original = (output / "menu_move.wav").read_bytes()
            generate_module.generate([self.entry], output, False, "model", "cpu", 100, 7, loader)
            self.assertEqual(loader.call_count, 1)
            generate_module.generate([self.entry], output, True, "model", "cpu", 100, 7, loader)
            self.assertEqual(loader.call_count, 2)
            self.assertEqual((output / "menu_move.wav").read_bytes(), original)
            self.assertEqual(list(output.glob(".*.wav")), [])

    def test_failed_force_keeps_existing_file(self):
        loader = mock.Mock(return_value=(mock.Mock(side_effect=RuntimeError("inference failed")), "cpu"))
        fake_generator = mock.Mock()
        fake_generator.manual_seed.return_value = fake_generator
        fake_torch = mock.Mock()
        fake_torch.Generator.return_value = fake_generator
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict("sys.modules", {"torch": fake_torch}):
            output = Path(directory)
            destination = output / "menu_move.wav"
            destination.write_bytes(b"old complete file")
            with self.assertRaisesRegex(RuntimeError, "inference failed"):
                generate_module.generate([self.entry], output, True, "model", "cpu", 100, 7, loader)
            self.assertEqual(destination.read_bytes(), b"old complete file")
            self.assertEqual(list(output.glob(".*.wav")), [])


if __name__ == "__main__":
    unittest.main()
