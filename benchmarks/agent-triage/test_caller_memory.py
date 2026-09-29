"""Fail-closed checks for the caller attribution sampler."""
import importlib.util
from pathlib import Path
import threading
import time
import unittest
from unittest.mock import patch, Mock

spec=importlib.util.spec_from_file_location('caller_memory',Path(__file__).with_name('caller-memory.py'))
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)


class SamplerChecks(unittest.TestCase):
    def make(self):
        sampler=m.Sampler.__new__(m.Sampler)
        sampler.roles={'7':'caller'};sampler.samples=[];sampler.phase='quiet';sampler.started=time.monotonic()
        sampler.identities={'7':123}
        return sampler

    def test_reject_pid_reuse_without_appending_sample(self):
        sampler=self.make()
        with patch.object(m,'proc_snapshot',return_value={'cpu':{'startTicks':456}}):
            with self.assertRaises(ValueError): sampler.sample()
        self.assertEqual(sampler.samples,[])

    def test_unreadable_process_is_not_silently_omitted(self):
        sampler=self.make()
        with patch.object(m,'proc_snapshot',side_effect=FileNotFoundError):
            with self.assertRaises(FileNotFoundError): sampler.sample()
        self.assertEqual(sampler.samples,[])

    def test_background_failure_is_reported_at_close(self):
        sampler=self.make();sampler.stop=threading.Event();sampler.thread=Mock();sampler.thread.is_alive.return_value=False
        sampler.errors=['smaps missing']
        with self.assertRaises(ValueError): sampler.close()
        self.assertTrue(sampler.stop.is_set())

    def test_quiet_samples_keep_identity_and_phase(self):
        sampler=self.make();value={'cpu':{'startTicks':123},'memoryKiB':{'Pss':1024},'threads':9}
        with patch.object(m,'proc_snapshot',return_value=value):sampler.sample()
        self.assertEqual(sampler.samples[0]['phase'],'quiet')
        self.assertEqual(sampler.samples[0]['processes']['7'],value)


if __name__=='__main__':unittest.main()
