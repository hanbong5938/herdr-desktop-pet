use crate::{HerdrIntegration, Route};
use desktop_pet_contracts::*;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub(crate) struct Source {
    pub id: u64,
    pub descriptor: SourceDescriptor,
    pub generation: u64,
    pub connected: bool,
    pub observed: bool,
    pub records: Vec<AgentRecord>,
    pub panes: Arc<[String]>,
    pub counts: SourceCounts,
}
pub(crate) struct Domain {
    pub instance: String,
    pub gate: bool,
    pub sources: BTreeMap<String, Source>,
    pub retired: HashMap<String, u64>,
    pub outcomes: Vec<OutcomeObservation>,
    pub completions: Vec<CompletionObservation>,
    pub invalidations: Vec<(String,u64,String,String)>,
    next_id: u64,
}
impl Default for Domain {
    fn default() -> Self { Self { instance:"test-instance".to_owned(), gate:true, sources:BTreeMap::new(), retired:HashMap::new(), outcomes:Vec::new(), completions:Vec::new(), invalidations:Vec::new(), next_id:1 } }
}
impl Domain {
    pub(crate) fn key(&self, source: &str, terminal: &str) -> SessionKey {
        let source = &self.sources[source];
        SessionKey { source_id:source.id, generation:source.generation, terminal_id:terminal.to_owned() }
    }
    fn resolve(&self, key: &SessionKey) -> Result<(&Source, &AgentRecord), PromptError> {
        let source = self.sources.values().find(|source| source.id == key.source_id).ok_or(PromptError::StaleTarget)?;
        if source.generation != key.generation { return Err(PromptError::StaleTarget); }
        let mut records = source.records.iter().filter(|record| record.terminal_id == key.terminal_id);
        let record = records.next().ok_or(PromptError::StaleTarget)?;
        if records.next().is_some() || source.records.iter().filter(|other| other.pane_id == record.pane_id).count() != 1 || source.panes.iter().filter(|pane| **pane == record.pane_id).count() != 1 { return Err(PromptError::StaleTarget); }
        Ok((source,record))
    }
}
impl DomainAccess for Domain {
    fn instance_id(&self) -> &str { &self.instance }
    fn mutation_allowed(&self) -> Result<(), String> { if self.gate { Ok(()) } else { Err("Prepare gate closed".to_owned()) } }
    fn source_generation(&self, source: &str) -> Option<u64> { self.sources.get(source).map(|source| source.generation) }
    fn retired_source_generation(&self, source: &str) -> Option<u64> { self.retired.get(source).copied() }
    fn source_descriptor(&self, source: &str) -> Option<&SourceDescriptor> { self.sources.get(source).map(|source| &source.descriptor) }
    fn source_admission(&self, source: &str, generation: u64) -> SourceAdmission {
        if generation == 0 || self.retired.get(source).is_some_and(|retired| *retired >= generation) || self.sources.get(source).is_some_and(|current| current.generation > generation) { SourceAdmission::Rejected }
        else if !self.sources.contains_key(source) && self.sources.len() >= MAX_SOURCES { SourceAdmission::Capacity }
        else { SourceAdmission::Allowed }
    }
    fn remove_offline_source(&mut self, source: &str, generation: u64) -> bool {
        if self.sources.get(source).is_some_and(|source| source.generation == generation && !source.connected) { self.remove_source(source,generation) } else { false }
    }
    fn begin_source(&mut self, descriptor: SourceDescriptor, generation: u64) -> bool {
        if self.source_admission(&descriptor.id,generation) != SourceAdmission::Allowed || self.sources.get(&descriptor.id).is_some_and(|source| source.generation >= generation) { return false; }
        let id = self.sources.get(&descriptor.id).map(|source| source.id).unwrap_or_else(|| { let id = self.next_id; self.next_id += 1; id });
        let counts=self.sources.get(&descriptor.id).map(|source|source.counts.disconnected()).unwrap_or_default();
        let observed=self.sources.get(&descriptor.id).is_none_or(|source|source.observed);
        self.outcomes.retain(|outcome|outcome.source != descriptor.id);
        self.completions.retain(|completion|completion.source != descriptor.id);
        self.sources.insert(descriptor.id.clone(), Source { id,descriptor,generation,connected:false,observed,records:Vec::new(),panes:Arc::from([]),counts });
        true
    }
    fn publish_source_snapshot(&mut self, source: &str, generation: u64, records: &mut dyn Iterator<Item=&AgentRecord>, counts: SourceCounts, pane_ids: Arc<[String]>, completions: &[(String,String)], observed_at: Instant) -> bool {
        let Some(current) = self.sources.get_mut(source).filter(|source| source.generation == generation) else { return false; };
        current.records.clear(); current.records.extend(records.cloned()); current.panes = pane_ids; current.counts = counts; current.connected = true;
        for (terminal,pane) in completions { self.completions.push(CompletionObservation { source:source.to_owned(),generation,terminal_id:terminal.clone(),pane_id:pane.clone(),observed_at }); }
        true
    }
    fn update_session_status(&mut self, update: SessionStatusUpdate<'_>) -> bool {
        let Some(source) = self.sources.get_mut(update.source).filter(|source| source.generation == update.generation && source.connected) else { return false; };
        if let Some(record) = source.records.iter_mut().find(|record| record.terminal_id == update.terminal_id && record.pane_id == update.pane_id) {
            record.status = update.status; source.counts = update.counts;
            if update.completion { self.completions.push(CompletionObservation { source:update.source.to_owned(),generation:update.generation,terminal_id:update.terminal_id.to_owned(),pane_id:update.pane_id.to_owned(),observed_at:update.observed_at }); }
            true
        } else { false }
    }
    fn mark_source_offline(&mut self, source: &str, generation: u64) -> bool {
        if let Some(current) = self.sources.get_mut(source).filter(|source| source.generation == generation) { current.connected = false; current.counts = current.counts.disconnected(); true } else { false }
    }
    fn remove_source(&mut self, source: &str, generation: u64) -> bool {
        if !self.sources.get(source).is_some_and(|source| source.generation == generation) { return false; }
        self.sources.remove(source); self.retired.entry(source.to_owned()).and_modify(|retired| *retired=(*retired).max(generation)).or_insert(generation); true
    }
    fn invalidate_session_outcome(&mut self, source: &str, generation: u64, terminal: &str, pane: &str) {
        self.invalidations.push((source.to_owned(),generation,terminal.to_owned(),pane.to_owned()));
    }
    fn push_outcome(&mut self, observation: OutcomeObservation) { self.outcomes.push(observation); }
    fn push_completion(&mut self, observation: CompletionObservation) { self.completions.push(observation); }
    fn prompt_target(&self, key: &SessionKey) -> Result<PromptTarget, PromptError> { self.prompt_route(key).map(|(target,_)| target) }
    fn prompt_route(&self, key: &SessionKey) -> Result<(PromptTarget,String), PromptError> {
        let (source,record) = self.resolve(key)?;
        if !source.descriptor.capabilities.prompt || source.descriptor.locality == SourceLocality::Remote { return Err(PromptError::ReadOnly); }
        if !source.observed { return Err(PromptError::StaleTarget); }
        if !source.connected { return Err(PromptError::Offline); }
        Ok((PromptTarget { source:source.descriptor.id.clone(),pane_id:record.pane_id.clone() },source.descriptor.provider.clone()))
    }
    fn worktree_remove_target(&self, key: &SessionKey) -> Result<WorktreeRemoveTarget, WorktreeRemoveTargetError> {
        let (source,record) = self.resolve(key).map_err(|error| match error { PromptError::Offline=>WorktreeRemoveTargetError::Offline, _=>WorktreeRemoveTargetError::Stale })?;
        if !source.descriptor.capabilities.remove_worktree || source.descriptor.locality == SourceLocality::Remote { return Err(WorktreeRemoveTargetError::ReadOnly); }
        if !source.observed { return Err(WorktreeRemoveTargetError::Stale); }
        if !source.connected { return Err(WorktreeRemoveTargetError::Offline); }
        let worktree = record.metadata.worktree.clone().filter(|worktree| worktree.is_linked_worktree).ok_or(WorktreeRemoveTargetError::NotLinkedWorktree)?;
        Ok(WorktreeRemoveTarget { key:key.clone(),source:source.descriptor.id.clone(),pane_id:record.pane_id.clone(),workspace_id:record.metadata.workspace_id.clone().ok_or(WorktreeRemoveTargetError::Stale)?,worktree })
    }
    fn set_source_label(&mut self, source: &str, generation: u64, label: &str) -> bool {
        if let Some(source) = self.sources.get_mut(source).filter(|source| source.generation == generation) { source.descriptor.label = label.to_owned(); true } else { false }
    }
    fn set_source_observed(&mut self, source: &str, generation: u64, observed: bool) -> bool {
        if let Some(source) = self.sources.get_mut(source).filter(|source| source.generation == generation) { source.observed = observed; true } else { false }
    }
}
#[derive(Default)]
pub(crate) struct Runtime { pub domain: Mutex<Domain>, pub wakes: AtomicUsize }
impl RuntimePort for Runtime {
    fn with_domain(&self, operation: &mut dyn FnMut(&mut dyn DomainAccess)) -> Result<(), String> {
        let mut domain = self.domain.lock().map_err(|_| "test domain is poisoned")?;
        operation(&mut *domain); Ok(())
    }
    fn wake_ui(&self) { self.wakes.fetch_add(1,Ordering::Relaxed); }
}
pub(crate) fn integration() -> (Arc<HerdrIntegration>,Arc<Runtime>) {
    let integration = HerdrIntegration::new();
    let runtime = Arc::new(Runtime::default());
    let port:Arc<dyn RuntimePort> = runtime.clone();
    assert!(integration.shared.runtime.set(port).is_ok());
    integration.set_enabled(true,&mut *runtime.domain.lock().unwrap());
    (integration,runtime)
}
pub(crate) fn record(terminal: &str, pane: &str) -> AgentRecord {
    AgentRecord { terminal_id:terminal.into(),pane_id:pane.into(),status:AgentStatus::Idle,metadata:SessionMetadata::default(),outcome_authoritative:false,outcome:None }
}
pub(crate) fn publish(integration: &HerdrIntegration, runtime: &Runtime, source: &str, generation: u64, route: Route, records: &[AgentRecord]) -> SessionKey {
    integration.shared.transaction(&mut |domain,registry| {
        let remote = matches!(route,Route::Remote(_));
        let descriptor = SourceDescriptor { id:source.into(),provider:"herdr".into(),label:source.into(),locality:if remote {SourceLocality::Remote} else {SourceLocality::Local},capabilities:SourceCapabilities {prompt:!remote,remove_worktree:!remote} };
        assert!(integration.shared.begin(domain,registry,descriptor,generation,route.clone()));
        assert!(domain.publish_source_snapshot(source,generation,&mut records.iter(),SourceCounts::default(),Arc::from(records.iter().map(|record|record.pane_id.clone()).collect::<Vec<_>>()),&[],Instant::now()));
        let binding = registry.bindings.get_mut(source).unwrap(); binding.connected = true; binding.offline_since = None;
    }).unwrap();
    runtime.domain.lock().unwrap().key(source,&records[0].terminal_id)
}
