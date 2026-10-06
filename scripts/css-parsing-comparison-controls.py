#!/usr/bin/env python3
"""Native public controls for CSS failure identity and honest loss accounting."""
import importlib.util
import json
import pathlib
import tempfile
import unittest
import hashlib
import sys

spec = importlib.util.spec_from_file_location('comparison', pathlib.Path(__file__).with_name('css-parsing-comparison.py'))
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


def summary(lines, total=20):
    return {'source_commit': 'a' * 40, 'selections': ['css-grid'], 'total': total,
            'files': 1, 'skipped_non_literal': 0, 'passed': total - sum(lines.values()),
            'failure_multiset': lines}


def computed(actual='Some("18px")', value='"calc(.5em + 10px)"', expected='"30px"', prop='flow-tolerance', file='/wpt/css/case.html'):
    return f'FAIL {file}: computed {prop}: {value} -> {actual}, expected {expected}'


class Controls(unittest.TestCase):
    def test_changed_failed_actual_stays_failed_without_loss(self):
        result = mod.compare(summary({computed('Some("calc(.5em + 10px)")'): 1}), summary({computed(): 1}))
        self.assertEqual((result['lost_count'], result['gained_count']), (0, 0))
        self.assertEqual(len(result['changed_failed_diagnostics']), 1)
        self.assertEqual(result['after_passed'], 19)

    def test_previously_passing_failure_remains_a_blocker(self):
        self.assertEqual(mod.compare(summary({}), summary({computed(): 1}))['lost_count'], 1)

    def test_changed_input_is_a_distinct_case(self):
        self.assertEqual(mod.compare(summary({computed(): 1}), summary({computed(value='"1px"'): 1}))['lost_count'], 1)

    def test_changed_expected_is_distinct(self):
        self.assertNotEqual(mod.identity(computed()), mod.identity(computed(expected='"31px"')))

    def test_property_identity_not_shared(self):
        self.assertNotEqual(mod.identity(computed()), mod.identity(computed(prop='width')))

    def test_file_identity_not_shared(self):
        self.assertNotEqual(mod.identity(computed()), mod.identity(computed(file='/wpt/css/other.html')))

    def test_escaped_quotes_and_backslashes_remain_input_identity(self):
        text = computed(value=r'"x\"y\\z"')
        self.assertEqual(json.loads(mod.identity(text))[3], r'"x\"y\\z"')

    def test_unicode_debug_escapes_do_not_need_json_reinterpretation(self):
        self.assertNotEqual(mod.identity(computed(value=r'"\u{80}"')), mod.identity(computed(value='"x"')))

    def test_expected_delimiter_inside_actual_does_not_change_key(self):
        self.assertEqual(mod.identity(computed('Some("a, expected b")')), mod.identity(computed()))

    def test_none_actual_and_some_actual_same_failed_identity(self):
        self.assertEqual(mod.identity(computed('None')), mod.identity(computed()))

    def test_duplicate_multiplicity_detects_one_lost_passing_variant(self):
        r = mod.compare(summary({computed(): 1}), summary({computed(): 2}))
        self.assertEqual(r['lost_count'], 1)

    def test_real_fail_to_pass_gain_preserved(self):
        self.assertEqual(mod.compare(summary({computed(): 1}), summary({}))['gained_count'], 1)

    def test_two_diagnostics_for_same_identity_keep_multiplicity(self):
        ids, _ = mod.failures(summary({computed('None'): 1, computed(): 1}))
        self.assertEqual(sum(ids.values()), 2)
        self.assertEqual(len(ids), 1)

    def test_valid_and_invalid_are_distinct_modes(self):
        self.assertNotEqual(mod.identity('FAIL /wpt/a: valid width: "1px"'), mod.identity('FAIL /wpt/a: invalid width: "1px" unexpectedly parsed'))

    def test_unknown_failure_format_aborts(self):
        with self.assertRaises(ValueError): mod.identity('FAIL unexplained compiler issue')

    def test_malformed_valid_suffix_aborts(self):
        with self.assertRaises(ValueError): mod.identity('FAIL /a: valid width: "1px" extra')

    def test_malformed_invalid_suffix_aborts(self):
        with self.assertRaises(ValueError): mod.identity('FAIL /a: invalid width: "1px" rejected')

    def test_missing_expected_aborts(self):
        with self.assertRaises(ValueError): mod.identity('FAIL /a: computed width: "1px" -> None')

    def test_unterminated_input_aborts(self):
        with self.assertRaises(ValueError): mod.identity('FAIL /a: valid width: "1px')

    def test_bool_multiplicity_aborts(self):
        with self.assertRaises(ValueError): mod.failures(summary({computed(): True}))

    def test_zero_and_negative_multiplicity_abort(self):
        for count in [0, -1]:
            with self.assertRaises(ValueError): mod.failures(summary({computed(): count}))

    def test_count_reconciliation_aborts(self):
        s = summary({computed(): 1}); s['passed'] += 1
        with self.assertRaises(ValueError): mod.failures(s)

    def test_each_cohort_dimension_change_aborts(self):
        for field, value in [('selections', ['css-color']), ('files', 2), ('total', 21), ('skipped_non_literal', 1)]:
            old=summary({}); new=summary({}); new[field]=value
            with self.assertRaises(ValueError): mod.compare(old, new)

    def test_hash_and_source_authentication_are_required(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=pathlib.Path(tmp)/'summary.json';p.write_text(json.dumps(summary({})))
            sha=hashlib.sha256(p.read_bytes()).hexdigest()
            self.assertEqual(mod.load(p,sha,'a'*40)['passed'],20)
            with self.assertRaises(ValueError): mod.load(p,'0'*64,'a'*40)
            with self.assertRaises(ValueError): mod.load(p,sha,'b'*40)

    def test_missing_cohort_cannot_be_treated_as_identical(self):
        old=summary({});new=summary({})
        del old['selections'];del new['selections']
        with self.assertRaises(ValueError):mod.compare(old,new)

    def test_invalid_cohort_metadata_aborts(self):
        for field,value in [('selections',[]),('files',True),('skipped_non_literal',-1)]:
            old=summary({});new=summary({});old[field]=new[field]=value
            with self.assertRaises(ValueError):mod.compare(old,new)

    def test_nonexact_hash_and_source_are_rejected(self):
        with self.assertRaises(ValueError):mod.load('/not-read','bad','a'*40)
        with self.assertRaises(ValueError):mod.load('/not-read','a'*64,'short')


if __name__ == '__main__':
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Controls)
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    if len(sys.argv) != 2:
        raise SystemExit('usage: css-parsing-comparison-controls.py COMPACT_OUTPUT')
    helper = pathlib.Path(__file__).with_name('css-parsing-comparison.py')
    compact = {'controls': result.testsRun, 'passed': result.testsRun - len(result.failures) - len(result.errors),
               'failed': len(result.failures) + len(result.errors),
               'failure_ids': [case.id() for case, _ in result.failures + result.errors],
               'helper_sha256': hashlib.sha256(helper.read_bytes()).hexdigest(),
               'controls_sha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()}
    pathlib.Path(sys.argv[1]).write_text(json.dumps(compact, sort_keys=True) + '\n')
    raise SystemExit(not result.wasSuccessful())
