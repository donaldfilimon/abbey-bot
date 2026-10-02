#!/usr/bin/env python3
import copy
import importlib.util
import itertools
import json
from pathlib import Path
import tempfile
import unittest
spec=importlib.util.spec_from_file_location('proposal',Path(__file__).with_name('check-deployment-proposal.py'))
proposal=importlib.util.module_from_spec(spec);spec.loader.exec_module(proposal)
class Tests(unittest.TestCase):
    def test_actual_template(self): self.assertEqual(proposal.errors(proposal.ROOT),[])
    def test_actual_expression_truth_table(self):
        jobs=json.loads((proposal.ROOT/proposal.TEMPLATE).read_bytes())['jobs']
        for repo,event,ref,head,result in itertools.product(['donaldfilimon/abbey-bot','foreign/repo'],['push','workflow_dispatch','pull_request','pull_request_target','workflow_run','issue_comment'],['refs/heads/main','refs/heads/topic'],['donaldfilimon/abbey-bot','foreign/repo'],['success','failure','cancelled','skipped']):
            for name in jobs:
                expression=jobs[name]['if']
                for key,value in [('github.event.pull_request.head.repo.full_name',head),('github.repository',repo),('github.event_name',event),('github.ref',ref),('needs.gate-macos.result',result)]: expression=expression.replace(key,repr(value))
                actual=eval(expression.replace('&&',' and ').replace('||',' or '),{'__builtins__':{}})
                trusted=repo=='donaldfilimon/abbey-bot'
                main_event=ref=='refs/heads/main' and event in ('push','workflow_dispatch')
                expected=trusted and (main_event or event=='pull_request' and head==repo) if name=='gate-macos' else trusted and main_event and result=='success'
                self.assertEqual(actual,expected,(name,repo,event,ref,head,result))
    def test_mutations_fail_closed(self):
        source=json.loads((proposal.ROOT/proposal.TEMPLATE).read_bytes())
        mutations=[lambda d:d['on'].update(workflow_run={}),lambda d:d['on']['push'].update(branches=['topic']),lambda d:d['on'].pop('workflow_dispatch'),lambda d:d['concurrency'].update({'cancel-in-progress':True}),lambda d:d['jobs']['deploy-macos']['concurrency'].update({'cancel-in-progress':True}),lambda d:d['jobs']['deploy-macos'].update(needs=[]),lambda d:d['jobs']['deploy-macos'].update({'if':'always()'}),lambda d:d['jobs']['gate-macos'].update({'if':'true'}),lambda d:d['jobs']['gate-macos'].update(name='Gate renamed'),lambda d:d['jobs']['deploy-macos'].update({'runs-on':['self-hosted']}),lambda d:d['jobs']['gate-macos'].update({'continue-on-error':True}),lambda d:d['jobs']['deploy-macos']['steps'].pop(-2),lambda d:d['jobs']['gate-macos']['steps'][5].update({'if':False}),lambda d:d['jobs']['gate-macos']['steps'][5].update({'continue-on-error':True}),lambda d:d['jobs']['deploy-macos']['steps'][5].update({'if':False}),lambda d:d['jobs']['deploy-macos']['steps'][5].update({'continue-on-error':True}),lambda d:d['jobs']['deploy-macos']['steps'][-2].update({'if':False})]
        for mutate in mutations:
            with tempfile.TemporaryDirectory() as temp:
                root=Path(temp);value=copy.deepcopy(source);mutate(value)
                path=root/proposal.TEMPLATE;path.parent.mkdir(parents=True);path.write_text(json.dumps(value))
                self.assertTrue(proposal.errors(root))
    def assert_rejected(self, value, message):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            path = root / proposal.TEMPLATE
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps(value))
            self.assertTrue(proposal.errors(root), message)

    def test_critical_step_mutations(self):
        source = json.loads((proposal.ROOT / proposal.TEMPLATE).read_bytes())
        checked = 0
        for job_name, job in source['jobs'].items():
            critical_names = [proposal.HOST_PREREQUISITES['name'],
                              proposal.AUDIT_INSTALL['name'], proposal.SOURCE_GATE['name']]
            if job_name == 'deploy-macos':
                critical_names += [proposal.QUALIFICATION['name'], proposal.QUALIFIED_INSTALL['name']]
            for step_name in critical_names:
                index = next(i for i, step in enumerate(job['steps']) if step.get('name') == step_name)
                mutations = {
                    'deleted': lambda steps: steps.pop(index),
                    'duplicated': lambda steps: steps.insert(index, copy.deepcopy(steps[index])),
                    'renamed': lambda steps: steps[index].update(name='replacement'),
                    'changed': lambda steps: steps[index].update(run='true'),
                    'skipped': lambda steps: steps[index].update({'if': False}),
                    'conditional': lambda steps: steps[index].update({'if': '${{ success() }}'}),
                    'ignored failure': lambda steps: steps[index].update({'continue-on-error': True}),
                    'shell override': lambda steps: steps[index].update(shell='sh {0}'),
                    'directory override': lambda steps: steps[index].update({'working-directory': 'other'}),
                    'environment override': lambda steps: steps[index].update(env={'PATH': '/tmp'}),
                    'or true': lambda steps: steps[index].update(run=steps[index]['run'] + ' || true'),
                    'masked last status': lambda steps: steps[index].update(run=steps[index]['run'] + '\ntrue'),
                    'errexit disabled': lambda steps: steps[index].update(run='set +e\n' + steps[index]['run']),
                    'pipeline': lambda steps: steps[index].update(run=steps[index]['run'] + ' | cat'),
                }
                for label, mutate in mutations.items():
                    with self.subTest(job=job_name, step=step_name, mutation=label):
                        value = copy.deepcopy(source)
                        mutate(value['jobs'][job_name]['steps'])
                        self.assert_rejected(value, label)
                        checked += 1
        self.assertEqual(checked, 112)

    def test_critical_step_ordering(self):
        source = json.loads((proposal.ROOT / proposal.TEMPLATE).read_bytes())
        for job_name in source['jobs']:
            for prerequisite in (proposal.HOST_PREREQUISITES, proposal.AUDIT_INSTALL):
                with self.subTest(job=job_name, prerequisite=prerequisite['name']):
                    value = copy.deepcopy(source)
                    steps = value['jobs'][job_name]['steps']
                    index = next(i for i, step in enumerate(steps) if step.get('name') == prerequisite['name'])
                    step = steps.pop(index)
                    gate = next(i for i, item in enumerate(steps) if item == proposal.SOURCE_GATE)
                    steps.insert(gate + 1, step)
                    self.assert_rejected(value, 'prerequisite after source gate')
        for first, second in ((proposal.SOURCE_GATE, proposal.QUALIFICATION),
                              (proposal.QUALIFICATION, proposal.QUALIFIED_INSTALL),
                              (proposal.SOURCE_GATE, proposal.QUALIFIED_INSTALL)):
            with self.subTest(first=first['name'], second=second['name']):
                value = copy.deepcopy(source)
                steps = value['jobs']['deploy-macos']['steps']
                left, right = steps.index(first), steps.index(second)
                steps[left], steps[right] = steps[right], steps[left]
                self.assert_rejected(value, 'deployment ordering')

    def test_activation_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp);path=root/proposal.TEMPLATE;path.parent.mkdir(parents=True);path.write_bytes((proposal.ROOT/proposal.TEMPLATE).read_bytes())
            active=root/'.github/workflows/rust-auto-deploy.yml';active.parent.mkdir(parents=True);active.write_bytes(path.read_bytes())
            self.assertIn('deployment proposal must remain dormant',proposal.errors(root))
if __name__=='__main__': unittest.main()
