//! Panel tab selection and the Agents list (`NodesPanelState`).

use eframe::egui;

use super::model::NodeData;

pub(crate) const WORKSPACE_MODULE_ENABLED: bool = false;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanelTab {
    Overview,
    Agents,
    Ollama,
    Settings,
}

/// One row in the Agents list (stable `id` for manifests and conversation loops).
#[derive(Clone)]
pub(crate) struct AgentRecord {
    pub(crate) id: usize,
    pub(crate) position: egui::Pos2,
    pub(crate) open: bool,
    pub(crate) data: super::model::NodeData,
}

pub struct NodesPanelState {
    pub(crate) next_agent_id: usize,
    pub(crate) agents: Vec<AgentRecord>,
    pub(crate) selected_add_kind: super::model::AgentNodeKind,
    pub(crate) active_tab: PanelTab,
}

impl Default for NodesPanelState {
    fn default() -> Self {
        Self {
            next_agent_id: 0,
            agents: Vec::new(),
            selected_add_kind: super::model::AgentNodeKind::Worker,
            active_tab: PanelTab::Overview,
        }
    }
}

impl NodesPanelState {
    pub(crate) fn add_agent(&mut self, kind: super::model::AgentNodeKind) {
        let id = self.next_agent_id;
        let data = match kind {
            super::model::AgentNodeKind::Manager => NodeData::new_manager(),
            super::model::AgentNodeKind::Worker => NodeData::new_worker(),
            super::model::AgentNodeKind::Evaluator => NodeData::new_evaluator(),
            super::model::AgentNodeKind::Researcher => NodeData::new_researcher(),
            super::model::AgentNodeKind::Topic => NodeData::new_topic(),
        };
        self.insert_agent_with_id(id, egui::pos2(0.0, 0.0), true, data);
    }

    pub(crate) fn insert_agent_with_id(
        &mut self,
        id: usize,
        pos: egui::Pos2,
        open: bool,
        data: NodeData,
    ) {
        self.agents.push(AgentRecord {
            id,
            position: pos,
            open,
            data,
        });
        if id + 1 > self.next_agent_id {
            self.next_agent_id = id + 1;
        }
    }

    pub(crate) fn remove_agent(&mut self, id: usize) {
        self.agents.retain(|a| a.id != id);
    }
}
