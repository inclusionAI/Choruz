//! Checkpointed reflective search. Selection never reads held-out test outcomes.
use crate::evaluation::{EvaluationCandidate, EvaluationSplit, EvaluationSuite};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationConfig {
    pub max_metric_calls: usize,
    pub max_proposals: usize,
    pub minibatch_size: usize,
    pub seed: u64,
    pub merge: bool,
    #[serde(default)]
    pub cache_evaluations: bool,
    #[serde(default)]
    pub evolve_team: bool,
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,
}

fn default_max_agents() -> usize {
    4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::{EvaluationCase, OutputCheck};

    fn setup() -> (EvaluationSuite, Optimization) {
        let suite = EvaluationSuite {
            name: "format".into(),
            cases: [
                EvaluationSplit::Train,
                EvaluationSplit::Validation,
                EvaluationSplit::Validation,
                EvaluationSplit::Test,
            ]
            .into_iter()
            .enumerate()
            .map(|(i, split)| EvaluationCase {
                id: i.to_string(),
                split,
                input: format!("task-{i}"),
                check: OutputCheck::Exact {
                    expected: "ok".into(),
                },
            })
            .collect(),
        };
        let config = OptimizationConfig {
            max_metric_calls: 64,
            max_proposals: 3,
            minibatch_size: 1,
            seed: 7,
            merge: true,
            cache_evaluations: false,
            evolve_team: false,
            max_agents: 4,
        };
        let seeds = ["left", "right"]
            .into_iter()
            .map(|instruction| EvaluationCandidate {
                revision_id: None,
                instruction: instruction.into(),
                team: None,
            })
            .collect();
        let state = Optimization::new(config, &suite, seeds).unwrap();
        (suite, state)
    }

    #[test]
    fn complementary_candidates_merge_from_training_and_test_only_after_selection() {
        let (suite, mut state) = setup();
        let mut merged = false;
        while let Some(action) = state.next_action(&suite).unwrap() {
            match action {
                SearchAction::Evaluate { candidate, case } => {
                    if suite.cases[case].split == EvaluationSplit::Test {
                        assert!(state.winner.is_some());
                        assert!(merged);
                    }
                    let correct = candidate >= 2
                        || (candidate == 0 && case == 1)
                        || (candidate == 1 && case == 2);
                    state
                        .complete_evaluation(&suite, if correct { "ok" } else { "wrong" }.into())
                        .unwrap();
                }
                SearchAction::Propose {
                    parents, component, ..
                } => {
                    assert!(matches!(component, Component::Instruction));
                    let input = state.proposal_input(&suite).unwrap().to_string();
                    assert!(input.contains("task-0"));
                    for forbidden in ["task-1", "task-2", "task-3"] {
                        assert!(!input.contains(forbidden));
                    }
                    assert_eq!(state.frontier(&suite), vec![0, 1]);
                    if parents.len() == 2 {
                        merged = true;
                        state.complete_proposal("combined".into()).unwrap();
                    } else {
                        state
                            .complete_proposal(
                                state.candidates[parents[0]].guidance.instruction.clone(),
                            )
                            .unwrap();
                    }
                }
            }
        }
        assert!(merged);
        assert_eq!(state.winner, Some(2));
        assert_eq!(state.candidates[2].parents.len(), 2);
        assert_eq!(state.candidates[2].origin, "merge");
        assert_eq!(state.frontier(&suite), vec![2]);
        assert_eq!(state.proposal_calls, 3);
        assert!(state.metric_calls <= state.config.max_metric_calls);
        assert_eq!(state.metric_calls, state.observations.len());
    }

    #[test]
    fn team_search_changes_members_and_preserves_executor_and_call_accounting() {
        for allowed in [false, true] {
            let (suite, mut state) = setup();
            state.config.evolve_team = allowed;
            state.config.max_agents = 3;
            let team = crate::team::Team {
                order: crate::team::Order::Parallel,
                members: vec![
                    crate::team::Member {
                        name: "derive".into(),
                        prompt: "Derive a candidate answer.".into(),
                    },
                    crate::team::Member {
                        name: "check".into(),
                        prompt: "Independently check the task constraints.".into(),
                    },
                ],
            };
            let mut calls = 0;
            let mut changed = false;
            while let Some(action) = state.next_action(&suite).unwrap() {
                match action {
                    SearchAction::Evaluate { candidate, .. } => {
                        let members = state.candidates[candidate]
                            .guidance
                            .team
                            .as_ref()
                            .map_or(0, |t| t.members.len());
                        calls += 1 + members;
                        state
                            .complete_evaluation(
                                &suite,
                                if members == 2 { "ok" } else { "wrong" }.into(),
                            )
                            .unwrap();
                    }
                    SearchAction::Propose {
                        component, parents, ..
                    } => {
                        calls += 1;
                        match component {
                            Component::Instruction => state
                                .complete_proposal("Keep the task constraints.".into())
                                .unwrap(),
                            Component::Team => {
                                assert!(allowed);
                                assert!(state.complete_proposal(r#"{"order":"parallel","members":[],"account":"other"}"#.into()).is_err());
                                let mut excessive = team.clone();
                                excessive.members.push(crate::team::Member {
                                    name: "extra".into(),
                                    prompt: "Extra call".into(),
                                });
                                assert!(
                                    state
                                        .complete_proposal(
                                            serde_json::to_string(&excessive).unwrap()
                                        )
                                        .is_err()
                                );
                                let instruction =
                                    state.candidates[parents[0]].guidance.instruction.clone();
                                state
                                    .complete_proposal(serde_json::to_string(&team).unwrap())
                                    .unwrap();
                                assert_eq!(
                                    state.candidates.last().unwrap().guidance.instruction,
                                    instruction
                                );
                                changed = true;
                            }
                        }
                    }
                }
            }
            assert_eq!(changed, allowed);
            assert_eq!(calls, state.model_calls_reserved);
            assert_eq!(state.can_apply(&suite), allowed);
            if allowed {
                assert_eq!(
                    state.candidates[state.winner.unwrap()].guidance.team,
                    Some(team)
                );
            }
        }
    }

    #[test]
    fn reservation_survives_checkpoint_without_refunding_or_repeating_a_call() {
        let (suite, mut state) = setup();
        state.next_action(&suite).unwrap().unwrap();
        let mut restored: Optimization = serde_json::from_value(json!(state)).unwrap();
        assert_eq!(restored.metric_calls, 1);
        assert!(restored.next_action(&suite).is_err());
        restored.complete_evaluation(&suite, "ok".into()).unwrap();
        assert!(restored.complete_evaluation(&suite, "ok".into()).is_err());
        let mut resumed: Optimization = serde_json::from_value(json!(restored)).unwrap();
        let action = resumed.next_action(&suite).unwrap().unwrap();
        assert_eq!(resumed.metric_calls, 2);
        assert!(matches!(
            action,
            SearchAction::Evaluate {
                candidate: 0,
                case: 2
            }
        ));
    }

    #[test]
    fn minimum_budget_preserves_final_comparison_and_never_uses_test_to_choose() {
        let (suite, state) = setup();
        let mut config = state.config;
        config.max_metric_calls = 6;
        let mut state = Optimization::new(
            config,
            &suite,
            state.candidates.into_iter().map(|c| c.guidance).collect(),
        )
        .unwrap();
        while let Some(action) = state.next_action(&suite).unwrap() {
            let SearchAction::Evaluate { candidate, case } = action else {
                panic!("No proposal budget remains after reserving the final test");
            };
            let correct = if suite.cases[case].split == EvaluationSplit::Test {
                candidate == 0
            } else {
                candidate == 1
            };
            state
                .complete_evaluation(&suite, if correct { "ok" } else { "wrong" }.into())
                .unwrap();
        }
        assert_eq!(state.winner, Some(1));
        assert_eq!(state.metric_calls, 6);
        assert_eq!(state.proposal_calls, 0);
        assert_eq!(state.mean(1, &[3]), Some(0.0));
    }

    #[test]
    fn fresh_comparisons_and_component_edits_preserve_budget_and_reject_equal_trials() {
        for cached in [false, true] {
            let (suite, mut state) = setup();
            state.config.cache_evaluations = cached;
            state.config.evolve_team = true;
            for candidate in &mut state.candidates {
                candidate.guidance.team =
                    Some(crate::team::Team::reviewer("Check formatting".into()));
            }
            let mut proposed_preflight = false;
            while let Some(action) = state.next_action(&suite).unwrap() {
                match action {
                    SearchAction::Evaluate { .. } => state
                        .complete_rollout(&suite, "wrong".into(), "Visible check".into())
                        .unwrap(),
                    SearchAction::Propose {
                        component, parents, ..
                    } => {
                        assert!(
                            state
                                .proposal_input(&suite)
                                .unwrap()
                                .to_string()
                                .contains("Visible check")
                        );
                        let parent = state.candidates[parents[0]].guidance.clone();
                        if matches!(component, Component::Team) {
                            assert!(state.complete_proposal("x".repeat(4_001)).is_err());
                        }
                        state
                            .complete_proposal(match component {
                                Component::Instruction => {
                                    format!("Revision {}", state.proposal_calls)
                                }
                                Component::Team => {
                                    serde_json::to_string(&crate::team::Team::reviewer(format!(
                                        "Revision {}",
                                        state.proposal_calls
                                    )))
                                    .unwrap()
                                }
                            })
                            .unwrap();
                        let child = &state.candidates.last().unwrap().guidance;
                        match component {
                            Component::Instruction => {
                                assert_eq!(child.team, parent.team)
                            }
                            Component::Team => {
                                proposed_preflight = true;
                                assert_eq!(child.instruction, parent.instruction);
                            }
                        }
                    }
                }
            }
            assert!(proposed_preflight);
            assert_eq!(state.winner, Some(0));
            assert_eq!(
                state.model_calls_reserved,
                2 * state.metric_calls + state.proposal_calls
            );
            assert!(state.observations.iter().all(|o| o.candidate < 2 || suite.cases[o.case].split == EvaluationSplit::Train));
            let parent_trials = state
                .observations
                .iter()
                .filter(|o| o.candidate == 0 && o.case == 0)
                .count();
            assert_eq!(parent_trials, if cached { 1 } else { 3 });
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Individual {
    pub guidance: EvaluationCandidate,
    pub parents: Vec<usize>,
    pub origin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub candidate: usize,
    pub case: usize,
    pub score: f64,
    pub output: String,
    pub preflight: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    Instruction,
    Team,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchAction {
    Evaluate {
        candidate: usize,
        case: usize,
    },
    Propose {
        parents: Vec<usize>,
        batch: Vec<usize>,
        component: Component,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Stage {
    Seeds,
    Choose,
    Reflect {
        parents: Vec<usize>,
        batch: Vec<usize>,
    },
    Trial {
        candidate: usize,
        parents: Vec<usize>,
        batch: Vec<usize>,
    },
    Validate,
    Final,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Optimization {
    pub config: OptimizationConfig,
    pub candidates: Vec<Individual>,
    pub observations: Vec<Observation>,
    pub metric_calls: usize,
    pub proposal_calls: usize,
    pub model_calls_reserved: usize,
    pub pending: Option<SearchAction>,
    pub winner: Option<usize>,
    stage: Stage,
    queue: Vec<(usize, usize)>,
    rounds: usize,
    rng: u64,
    train_order: Vec<usize>,
    train_cursor: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationSettings {
    pub suite: EvaluationSuite,
    pub config: OptimizationConfig,
    pub auto_apply: bool,
}

impl OptimizationSettings {
    pub fn validate(&self) -> Result<(), String> {
        let seed = EvaluationCandidate {
            revision_id: None,
            instruction: String::new(),
            team: None,
        };
        Optimization::new(self.config.clone(), &self.suite, vec![seed.clone(), seed]).map(|_| ())
    }
}

fn cases(suite: &EvaluationSuite, split: EvaluationSplit) -> Vec<usize> {
    suite
        .cases
        .iter()
        .enumerate()
        .filter_map(|(i, c)| (c.split == split).then_some(i))
        .collect()
}

impl Optimization {
    /// Test results may reject application, never choose or revise the winner.
    pub fn application_scores(&self, suite: &EvaluationSuite) -> Option<[f64; 4]> {
        if !matches!(self.stage, Stage::Complete) || self.pending.is_some() {
            return None;
        }
        let winner = self.winner?;
        let validation = cases(suite, EvaluationSplit::Validation);
        let test = cases(suite, EvaluationSplit::Test);
        Some([
            self.mean(0, &validation)?,
            self.mean(winner, &validation)?,
            self.mean(0, &test)?,
            self.mean(winner, &test)?,
        ])
    }

    pub fn can_apply(&self, suite: &EvaluationSuite) -> bool {
        self.application_scores(suite).is_some_and(
            |[base_validation, winner_validation, base_test, winner_test]| {
                winner_validation > base_validation && winner_test >= base_test
            },
        )
    }

    pub fn new(
        config: OptimizationConfig,
        suite: &EvaluationSuite,
        seeds: Vec<EvaluationCandidate>,
    ) -> Result<Self, String> {
        suite.validate()?;
        if !(1..=4).contains(&config.max_agents) {
            return Err("Maximum total agents must be between 1 and 4".into());
        }
        for seed in &seeds {
            if let Some(team) = &seed.team {
                team.validate(config.max_agents)?;
            }
        }
        let train = cases(suite, EvaluationSplit::Train);
        let validation = cases(suite, EvaluationSplit::Validation);
        let test = cases(suite, EvaluationSplit::Test);
        if seeds.len() != 2
            || !(1..=32).contains(&config.max_proposals)
            || !(1..=train.len()).contains(&config.minibatch_size)
            || config.max_metric_calls < 2 * (validation.len() + test.len())
            || config.max_metric_calls > 512
        {
            return Err("Provide two seeds and bounded search budgets covering validation and final test comparisons".into());
        }
        let queue = (0..seeds.len())
            .flat_map(|candidate| validation.iter().map(move |case| (candidate, *case)))
            .collect();
        let mut state = Self {
            rng: config.seed.max(1),
            config,
            candidates: seeds
                .into_iter()
                .map(|guidance| Individual {
                    guidance,
                    parents: vec![],
                    origin: "seed".into(),
                })
                .collect(),
            observations: vec![],
            metric_calls: 0,
            proposal_calls: 0,
            model_calls_reserved: 0,
            pending: None,
            winner: None,
            stage: Stage::Seeds,
            queue,
            rounds: 0,
            train_order: train,
            train_cursor: 0,
        };
        state.shuffle();
        Ok(state)
    }

    // This generator only makes experiment ordering reproducible, not secret.
    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn shuffle(&mut self) {
        for i in (1..self.train_order.len()).rev() {
            let j = self.random() as usize % (i + 1);
            self.train_order.swap(i, j);
        }
        self.train_cursor = 0;
    }

    fn observation(&self, candidate: usize, case: usize) -> Option<&Observation> {
        self.observations
            .iter()
            .rev()
            .find(|o| o.candidate == candidate && o.case == case)
    }

    fn scores(&self, candidate: usize, cases: &[usize]) -> Option<Vec<f64>> {
        cases
            .iter()
            .map(|case| self.observation(candidate, *case).map(|o| o.score))
            .collect()
    }

    fn mean(&self, candidate: usize, cases: &[usize]) -> Option<f64> {
        self.scores(candidate, cases)
            .map(|scores| scores.iter().sum::<f64>() / scores.len() as f64)
    }

    pub fn frontier(&self, suite: &EvaluationSuite) -> Vec<usize> {
        let validation = cases(suite, EvaluationSplit::Validation);
        let measured: Vec<_> = (0..self.candidates.len())
            .filter_map(|i| self.scores(i, &validation).map(|s| (i, s)))
            .collect();
        measured
            .iter()
            .filter(|(i, scores)| {
                !measured.iter().any(|(j, other)| {
                    j != i
                        && other.iter().zip(scores).all(|(a, b)| a >= b)
                        && (other.iter().zip(scores).any(|(a, b)| a > b)
                            || (other == scores && j < i))
                })
            })
            .map(|(i, _)| *i)
            .collect()
    }

    fn batch(&mut self) -> Vec<usize> {
        let mut batch = Vec::new();
        while batch.len() < self.config.minibatch_size {
            if self.train_cursor == self.train_order.len() {
                self.shuffle();
            }
            let case = self.train_order[self.train_cursor];
            self.train_cursor += 1;
            if !batch.contains(&case) {
                batch.push(case);
            }
        }
        batch
    }

    /// Reserve before dispatch and persist the returned state with the worker's
    /// lease. A pending action cannot be silently dispatched a second time.
    pub fn next_action(&mut self, suite: &EvaluationSuite) -> Result<Option<SearchAction>, String> {
        if self.pending.is_some() {
            return Err("A search action is already reserved".into());
        }
        let validation = cases(suite, EvaluationSplit::Validation);
        let test = cases(suite, EvaluationSplit::Test);
        loop {
            while !self.queue.is_empty() {
                let (candidate, case) = self.queue.remove(0);
                if self.config.cache_evaluations && self.observation(candidate, case).is_some() {
                    continue;
                }
                if self.metric_calls >= self.config.max_metric_calls {
                    return Err("Evaluation budget exhausted before the final comparison".into());
                }
                self.metric_calls += 1;
                self.model_calls_reserved += 1 + self.candidates[candidate]
                    .guidance
                    .team
                    .as_ref()
                    .map_or(0, |team| team.members.len());
                let action = SearchAction::Evaluate { candidate, case };
                self.pending = Some(action.clone());
                return Ok(Some(action));
            }
            match self.stage.clone() {
                Stage::Seeds | Stage::Validate => self.stage = Stage::Choose,
                Stage::Choose => {
                    let reserve = 2 * test.len();
                    let round_cost = 3 * self.config.minibatch_size + validation.len();
                    if self.proposal_calls >= self.config.max_proposals
                        || self.rounds >= 4 * self.config.max_proposals
                        || self.metric_calls + round_cost + reserve > self.config.max_metric_calls
                    {
                        let winner = (0..self.candidates.len())
                            .filter_map(|i| self.mean(i, &validation).map(|s| (i, s)))
                            .max_by(|(i, a), (j, b)| a.total_cmp(b).then_with(|| j.cmp(i)))
                            .map(|(i, _)| i)
                            .ok_or("No fully validated candidate")?;
                        self.winner = Some(winner);
                        self.queue = [0, winner]
                            .into_iter()
                            .enumerate()
                            .filter_map(|(i, c)| (i == 0 || c != 0).then_some(c))
                            .flat_map(|c| test.iter().map(move |t| (c, *t)))
                            .collect();
                        self.stage = Stage::Final;
                        continue;
                    }
                    self.rounds += 1;
                    let frontier = self.frontier(suite);
                    let parent = frontier[self.random() as usize % frontier.len()];
                    let mut parents = vec![parent];
                    if self.config.merge && self.rounds.is_multiple_of(3) && frontier.len() > 1 {
                        let alternatives: Vec<_> =
                            frontier.into_iter().filter(|i| *i != parent).collect();
                        parents.push(alternatives[self.random() as usize % alternatives.len()]);
                    }
                    let batch = self.batch();
                    self.queue = parents
                        .iter()
                        .flat_map(|p| batch.iter().map(move |c| (*p, *c)))
                        .collect();
                    self.stage = Stage::Reflect { parents, batch };
                }
                Stage::Reflect { parents, batch } => {
                    if parents.iter().any(|p| self.mean(*p, &batch) == Some(1.0)) {
                        self.stage = Stage::Choose;
                        continue;
                    }
                    let component = if self.config.evolve_team && self.proposal_calls % 2 == 1 {
                        Component::Team
                    } else {
                        Component::Instruction
                    };
                    self.proposal_calls += 1;
                    self.model_calls_reserved += 1;
                    let action = SearchAction::Propose {
                        parents,
                        batch,
                        component,
                    };
                    self.pending = Some(action.clone());
                    return Ok(Some(action));
                }
                Stage::Trial {
                    candidate,
                    parents,
                    batch,
                } => {
                    let child = self
                        .mean(candidate, &batch)
                        .ok_or("Missing trial results")?;
                    let parent = parents
                        .iter()
                        .filter_map(|p| self.mean(*p, &batch))
                        .fold(0.0, f64::max);
                    if child > parent {
                        self.queue = validation.iter().map(|c| (candidate, *c)).collect();
                        self.stage = Stage::Validate;
                    } else {
                        self.stage = Stage::Choose;
                    }
                }
                Stage::Final => self.stage = Stage::Complete,
                Stage::Complete => return Ok(None),
            }
        }
    }

    pub fn complete_evaluation(
        &mut self,
        suite: &EvaluationSuite,
        output: String,
    ) -> Result<(), String> {
        self.complete_rollout(suite, output, String::new())
    }

    pub fn complete_rollout(
        &mut self,
        suite: &EvaluationSuite,
        output: String,
        preflight: String,
    ) -> Result<(), String> {
        let Some(SearchAction::Evaluate { candidate, case }) = self.pending.clone() else {
            return Err("No evaluation is reserved".into());
        };
        if output.len() > 16_000 || preflight.len() > 16_000 {
            return Err("Evaluation output exceeds its limit".into());
        }
        self.observations.push(Observation {
            candidate,
            case,
            score: suite.cases[case].check.score(&output),
            output,
            preflight,
        });
        self.pending = None;
        Ok(())
    }

    pub fn proposal_input(&self, suite: &EvaluationSuite) -> Result<Value, String> {
        let Some(SearchAction::Propose {
            parents,
            batch,
            component,
        }) = &self.pending
        else {
            return Err("No proposal is reserved".into());
        };
        if batch
            .iter()
            .any(|i| suite.cases[*i].split != EvaluationSplit::Train)
        {
            return Err("Reflection accepts training evidence only".into());
        }
        Ok(
            json!({"component":component,"max_agents":self.config.max_agents,"operation":if parents.len()==2 {"merge"} else {"reflect"},
            "parents":parents.iter().map(|p| json!({"guidance":self.candidates[*p].guidance,
                "examples":batch.iter().map(|i| json!({"input":suite.cases[*i].input,"check":suite.cases[*i].check,
                    "observed":self.observation(*p,*i)})).collect::<Vec<_>>() })).collect::<Vec<_>>() }),
        )
    }

    pub fn complete_proposal(&mut self, text: String) -> Result<(), String> {
        let Some(SearchAction::Propose {
            parents,
            batch,
            component,
        }) = self.pending.clone()
        else {
            return Err("No proposal is reserved".into());
        };
        let limit = match component {
            Component::Instruction => 12_000,
            Component::Team => 16_000,
        };
        if text.trim().is_empty() || text.len() > limit {
            return Err("Proposed component is empty or exceeds its limit".into());
        }
        let mut guidance = self.candidates[parents[0]].guidance.clone();
        guidance.revision_id = None;
        match component {
            Component::Instruction => guidance.instruction = text,
            Component::Team => {
                let team: Option<crate::team::Team> = serde_json::from_str(&text)
                    .map_err(|_| "Proposed team must be a team object or null")?;
                if let Some(team) = &team {
                    team.validate(self.config.max_agents)?;
                }
                guidance.team = team.filter(|team| !team.members.is_empty());
            }
        };
        self.pending = None;
        if self.candidates.iter().any(|c| {
            c.guidance.instruction == guidance.instruction && c.guidance.team == guidance.team
        }) {
            self.stage = Stage::Choose;
            return Ok(());
        }
        let candidate = self.candidates.len();
        self.candidates.push(Individual {
            guidance,
            origin: if parents.len() == 2 {
                "merge".into()
            } else {
                "reflection".into()
            },
            parents: parents.clone(),
        });
        self.queue = batch.iter().map(|c| (candidate, *c)).collect();
        self.stage = Stage::Trial {
            candidate,
            parents,
            batch,
        };
        Ok(())
    }
}
