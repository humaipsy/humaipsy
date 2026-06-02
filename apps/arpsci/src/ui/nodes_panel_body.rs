//! Per-node editor UI for agent rows.

use eframe::egui;

use crate::agents::nodes_panel::{
    AgentNodeKind, AgentRecord, EvaluatorAgentsPick, NodePayload, TOPIC_PRESETS,
};

const CUSTOM_TOPIC_LABEL: &str = "Custom";

pub fn show_node_body(id: usize, ui: &mut egui::Ui, agents: &mut [AgentRecord]) {
    let Some(idx) = agents.iter().position(|a| a.id == id) else {
        return;
    };
    match agents[idx].data.kind {
        AgentNodeKind::Manager => {
            // Make manager node slightly wider than others.
            {
                let manager_data = match &mut agents[idx].data.payload {
                    NodePayload::Manager(m) => m,
                    _ => unreachable!("kind mismatch"),
                };
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.add(egui::TextEdit::singleline(&mut manager_data.name));
                    });
                });
            }
        }
        AgentNodeKind::Worker => {
            let mut managers: Vec<(usize, String)> = agents
                .iter()
                .filter_map(|a| {
                    if let NodePayload::Manager(m) = &a.data.payload {
                        Some((a.id, m.name.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            managers.sort_by(|a, b| a.1.cmp(&b.1));

            let mut workers: Vec<(usize, String)> = agents
                .iter()
                .filter_map(|a| {
                    if let NodePayload::Worker(w) = &a.data.payload {
                        Some((a.id, w.name.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            workers.sort_by(|a, b| a.1.cmp(&b.1));

            let my_manager_node = match &agents[idx].data.payload {
                NodePayload::Worker(w) => w.manager_node,
                _ => None,
            };
            let my_partner_worker = match &agents[idx].data.payload {
                NodePayload::Worker(w) => w.partner_worker,
                _ => None,
            };

            let mut pending_manager_pick: Option<Option<usize>> = None;
            let mut pending_partner_pick: Option<Option<usize>> = None;
            {
                let worker_data = match &mut agents[idx].data.payload {
                    NodePayload::Worker(w) => w,
                    _ => unreachable!("kind mismatch"),
                };

                ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label("Manager:");
                            let selected_text = my_manager_node
                                .and_then(|mid| {
                                    managers
                                        .iter()
                                        .find(|(id, _)| *id == mid)
                                        .map(|(_, n)| n.clone())
                                })
                                .unwrap_or_else(|| "Unassigned".to_string());
                            egui::ComboBox::from_id_salt(ui.id().with(id).with("manager_pick"))
                                .selected_text(selected_text)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(my_manager_node.is_none(), "Unassigned")
                                        .clicked()
                                    {
                                        pending_manager_pick = Some(None);
                                    }
                                    for &(mgr_id, ref mgr_name) in &managers {
                                        if ui
                                            .selectable_label(
                                                my_manager_node == Some(mgr_id),
                                                mgr_name.as_str(),
                                            )
                                            .clicked()
                                        {
                                            pending_manager_pick = Some(Some(mgr_id));
                                        }
                                    }
                                });
                        });

                        ui.horizontal(|ui| {
                            ui.label("Pair with:");
                            let selected_text = my_partner_worker
                                .and_then(|pid| {
                                    workers
                                        .iter()
                                        .find(|(wid, _)| *wid == pid)
                                        .map(|(_, n)| n.clone())
                                })
                                .unwrap_or_else(|| "Auto".to_string());
                            egui::ComboBox::from_id_salt(ui.id().with(id).with("partner_pick"))
                                .selected_text(selected_text)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(my_partner_worker.is_none(), "Auto")
                                        .clicked()
                                    {
                                        pending_partner_pick = Some(None);
                                    }
                                    for &(worker_id, ref worker_name) in &workers {
                                        if worker_id == id {
                                            continue;
                                        }
                                        if ui
                                            .selectable_label(
                                                my_partner_worker == Some(worker_id),
                                                worker_name.as_str(),
                                            )
                                            .clicked()
                                        {
                                            pending_partner_pick = Some(Some(worker_id));
                                        }
                                    }
                                });
                        });

                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing = egui::Vec2::new(5.0, 2.0);

                            ui.horizontal(|ui| {
                                ui.label("Name:");
                                ui.add(egui::TextEdit::singleline(&mut worker_data.name));
                            });

                            ui.horizontal(|ui| {
                                ui.label("Instruction:");
                                egui::ComboBox::from_id_salt(
                                    ui.id().with(id).with("instruction_mode"),
                                )
                                .selected_text(if worker_data.instruction_mode.is_empty() {
                                    "Select".to_string()
                                } else {
                                    worker_data.instruction_mode.clone()
                                })
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(
                                            worker_data.instruction_mode == "Assistant",
                                            "Assistant",
                                        )
                                        .clicked()
                                    {
                                        worker_data.instruction_mode = "Assistant".to_string();
                                        worker_data.instruction = "You are a helpful assistant. Answer clearly, stay concise, and focus on the user request.".to_string();
                                    }
                                    if ui
                                        .selectable_label(
                                            worker_data.instruction_mode == "Math Teacher",
                                            "Math Teacher",
                                        )
                                        .clicked()
                                    {
                                        worker_data.instruction_mode = "Math Teacher".to_string();
                                    }
                                    if ui
                                        .selectable_label(
                                            worker_data.instruction_mode == "Debate",
                                            "Debate",
                                        )
                                        .clicked()
                                    {
                                        worker_data.instruction_mode = "Debate".to_string();
                                    }
                                });
                            });

                            ui.horizontal(|ui| {
                                ui.label("Instruction:");
                                ui.add(egui::TextEdit::singleline(&mut worker_data.instruction));
                            });
                        });

                        ui.separator();

                        ui.horizontal(|ui| {
                            ui.label("Topic:");
                            egui::ComboBox::from_id_salt(
                                ui.id().with(id).with("worker_topic_analysis_mode"),
                            )
                            .selected_text(if worker_data.analysis_mode.is_empty() {
                                "Select".to_string()
                            } else {
                                worker_data.analysis_mode.clone()
                            })
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        worker_data.analysis_mode == CUSTOM_TOPIC_LABEL,
                                        CUSTOM_TOPIC_LABEL,
                                    )
                                    .clicked()
                                {
                                    worker_data.analysis_mode = CUSTOM_TOPIC_LABEL.to_string();
                                    worker_data.conversation_topic_source = "Own".to_string();
                                }
                                for &(label, sentence) in TOPIC_PRESETS {
                                    if ui
                                        .selectable_label(worker_data.analysis_mode == label, label)
                                        .clicked()
                                    {
                                        worker_data.analysis_mode = label.to_string();
                                        worker_data.conversation_topic = sentence.to_string();
                                        worker_data.conversation_topic_source = "Preset".to_string();
                                    }
                                }
                            });
                        });
                        ui.horizontal(|ui| {
                            ui.label("Topic:");
                            let response = ui.add(egui::TextEdit::singleline(
                                &mut worker_data.conversation_topic,
                            ));
                            if response.changed() {
                                worker_data.analysis_mode = CUSTOM_TOPIC_LABEL.to_string();
                                worker_data.conversation_topic_source = "Own".to_string();
                            }
                        });
                    });
            }

            if let Some(pick) = pending_manager_pick {
                if let NodePayload::Worker(w) = &mut agents[idx].data.payload {
                    w.manager_node = pick;
                }
            }
            if let Some(pick) = pending_partner_pick {
                if let NodePayload::Worker(w) = &mut agents[idx].data.payload {
                    w.partner_worker = pick;
                }
            }
        }
        AgentNodeKind::Evaluator => {
            let mut managers: Vec<(usize, String)> = agents
                .iter()
                .filter_map(|a| {
                    if let NodePayload::Manager(m) = &a.data.payload {
                        Some((a.id, m.name.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            managers.sort_by(|a, b| a.1.cmp(&b.1));

            let mut workers: Vec<(usize, String)> = agents
                .iter()
                .filter_map(|a| {
                    if let NodePayload::Worker(w) = &a.data.payload {
                        Some((a.id, w.name.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            workers.sort_by(|a, b| a.1.cmp(&b.1));

            let (my_manager_node, my_eval_worker_pin) = match &agents[idx].data.payload {
                NodePayload::Evaluator(e) => (e.manager_node, e.worker_node),
                _ => (None, None),
            };

            let mut pending_manager_pick: Option<Option<usize>> = None;
            let mut pending_agents_pick: Option<EvaluatorAgentsPick> = None;

            {
                let evaluator_data = match &mut agents[idx].data.payload {
                    NodePayload::Evaluator(e) => e,
                    _ => unreachable!("kind mismatch"),
                };

                ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label("Manager:");
                            let selected_text = my_manager_node
                                .and_then(|mid| {
                                    managers
                                        .iter()
                                        .find(|(id, _)| *id == mid)
                                        .map(|(_, n)| n.clone())
                                })
                                .unwrap_or_else(|| "Unassigned".to_string());
                            egui::ComboBox::from_id_salt(
                                ui.id().with(id).with("eval_manager_pick"),
                            )
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(my_manager_node.is_none(), "Unassigned")
                                    .clicked()
                                {
                                    pending_manager_pick = Some(None);
                                }
                                for &(mgr_id, ref mgr_name) in &managers {
                                    if ui
                                        .selectable_label(
                                            my_manager_node == Some(mgr_id),
                                            mgr_name.as_str(),
                                        )
                                        .clicked()
                                    {
                                        pending_manager_pick = Some(Some(mgr_id));
                                    }
                                }
                            });
                        });

                        ui.horizontal(|ui| {
                            ui.label("Agents to Evaluate:");
                            let selected_text = if evaluator_data.evaluate_all_workers {
                                "All Workers".to_string()
                            } else {
                                my_eval_worker_pin
                                    .and_then(|wid| {
                                        workers
                                            .iter()
                                            .find(|(id, _)| *id == wid)
                                            .map(|(_, n)| n.clone())
                                    })
                                    .unwrap_or_else(|| "Unassigned".to_string())
                            };
                            egui::ComboBox::from_id_salt(
                                ui.id().with(id).with("eval_agents_pick"),
                            )
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                let unassigned_sel = !evaluator_data.evaluate_all_workers
                                    && my_eval_worker_pin.is_none();
                                if ui.selectable_label(unassigned_sel, "Unassigned").clicked() {
                                    pending_agents_pick = Some(EvaluatorAgentsPick::Unassigned);
                                }
                                let all_sel = evaluator_data.evaluate_all_workers;
                                if ui.selectable_label(all_sel, "All Workers").clicked() {
                                    pending_agents_pick = Some(EvaluatorAgentsPick::AllWorkers);
                                }
                                for &(worker_id, ref worker_name) in &workers {
                                    let sel = !evaluator_data.evaluate_all_workers
                                        && my_eval_worker_pin == Some(worker_id);
                                    if ui
                                        .selectable_label(sel, worker_name.as_str())
                                        .clicked()
                                    {
                                        pending_agents_pick =
                                            Some(EvaluatorAgentsPick::Worker(worker_id));
                                    }
                                }
                            });
                        });

                        ui.separator();

                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.add(egui::TextEdit::singleline(&mut evaluator_data.name));
                        });

                        ui.horizontal(|ui| {
                            ui.label("Analysis:");
                            egui::ComboBox::from_id_salt(
                                ui.id().with(id).with("eval_analysis_mode"),
                            )
                            .selected_text(
                                if evaluator_data.analysis_mode.is_empty() {
                                    "Select".to_string()
                                } else {
                                    evaluator_data.analysis_mode.clone()
                                },
                            )
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        evaluator_data.analysis_mode == "Topic Extraction",
                                        "Topic Extraction",
                                    )
                                    .clicked()
                                {
                                    evaluator_data.analysis_mode =
                                        "Topic Extraction".to_string();
                                    evaluator_data.instruction = "Topic Extraction: extract the topic in 1 or 2 words. Identify what is the topic of the sentence being analysed.".to_string();
                                }
                                if ui
                                    .selectable_label(
                                        evaluator_data.analysis_mode == "Decision Analysis",
                                        "Decision Analysis",
                                    )
                                    .clicked()
                                {
                                    evaluator_data.analysis_mode =
                                        "Decision Analysis".to_string();
                                    evaluator_data.instruction = "Decision Analysis: extract a decision in 1 or 2 sentences about the agent in the message being analysed. Focus on the concrete decision and its intent.".to_string();
                                }
                                if ui
                                    .selectable_label(
                                        evaluator_data.analysis_mode
                                            == "Sentiment Classification",
                                        "Sentiment Classification",
                                    )
                                    .clicked()
                                {
                                    evaluator_data.analysis_mode =
                                        "Sentiment Classification".to_string();
                                    evaluator_data.instruction = "Sentiment Classification: extract the sentiment of the message being analysed and return one word that is the sentiment.".to_string();
                                }
                            });
                        });

                        ui.horizontal(|ui| {
                            ui.label("Instruction:");
                            ui.add(egui::TextEdit::singleline(&mut evaluator_data.instruction));
                        });

                        ui.horizontal(|ui| {
                            if ui
                                .checkbox(&mut evaluator_data.limit_token, "Limit Token")
                                .changed()
                            {
                                if !evaluator_data.limit_token {
                                    evaluator_data.num_predict.clear();
                                }
                            }
                            if evaluator_data.limit_token {
                                ui.label("num_predict:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut evaluator_data.num_predict)
                                        .desired_width(80.0),
                                );
                            }
                        });
                    });
            }

            if let Some(pick) = pending_manager_pick {
                if let NodePayload::Evaluator(e) = &mut agents[idx].data.payload {
                    e.manager_node = pick;
                }
            }
            if let Some(pick) = pending_agents_pick {
                if let NodePayload::Evaluator(e) = &mut agents[idx].data.payload {
                    match pick {
                        EvaluatorAgentsPick::Unassigned => {
                            e.evaluate_all_workers = false;
                            e.worker_node = None;
                            e.active = false;
                        }
                        EvaluatorAgentsPick::AllWorkers => {
                            e.evaluate_all_workers = true;
                            e.worker_node = None;
                            e.active = true;
                        }
                        EvaluatorAgentsPick::Worker(wid) => {
                            e.evaluate_all_workers = false;
                            e.worker_node = Some(wid);
                            e.active = true;
                        }
                    }
                }
            }
        }
        AgentNodeKind::Researcher => {
            let mut managers: Vec<(usize, String)> = agents
                .iter()
                .filter_map(|a| {
                    if let NodePayload::Manager(m) = &a.data.payload {
                        Some((a.id, m.name.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            managers.sort_by(|a, b| a.1.cmp(&b.1));

            let mut workers: Vec<(usize, String)> = agents
                .iter()
                .filter_map(|a| {
                    if let NodePayload::Worker(w) = &a.data.payload {
                        Some((a.id, w.name.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            workers.sort_by(|a, b| a.1.cmp(&b.1));

            let (my_manager_node, my_injection_worker) = match &agents[idx].data.payload {
                NodePayload::Researcher(r) => (r.manager_node, r.worker_node),
                _ => (None, None),
            };

            let mut pending_manager_pick: Option<Option<usize>> = None;
            let mut pending_injection_pick: Option<Option<usize>> = None;
            {
                let researcher_data = match &mut agents[idx].data.payload {
                    NodePayload::Researcher(r) => r,
                    _ => unreachable!("kind mismatch"),
                };

                ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label("Manager:");
                            let selected_text = my_manager_node
                                .and_then(|mid| {
                                    managers
                                        .iter()
                                        .find(|(id, _)| *id == mid)
                                        .map(|(_, n)| n.clone())
                                })
                                .unwrap_or_else(|| "Unassigned".to_string());
                            egui::ComboBox::from_id_salt(
                                ui.id().with(id).with("researcher_manager_pick"),
                            )
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(my_manager_node.is_none(), "Unassigned")
                                    .clicked()
                                {
                                    pending_manager_pick = Some(None);
                                }
                                for &(mgr_id, ref mgr_name) in &managers {
                                    if ui
                                        .selectable_label(
                                            my_manager_node == Some(mgr_id),
                                            mgr_name.as_str(),
                                        )
                                        .clicked()
                                    {
                                        pending_manager_pick = Some(Some(mgr_id));
                                    }
                                }
                            });
                        });

                        ui.horizontal(|ui| {
                            ui.label("Injection:");
                            let selected_text = my_injection_worker
                                .and_then(|wid| {
                                    workers
                                        .iter()
                                        .find(|(id, _)| *id == wid)
                                        .map(|(_, n)| n.clone())
                                })
                                .unwrap_or_else(|| "Unassigned".to_string());
                            egui::ComboBox::from_id_salt(
                                ui.id().with(id).with("researcher_injection_pick"),
                            )
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        my_injection_worker.is_none(),
                                        "Unassigned",
                                    )
                                    .clicked()
                                {
                                    pending_injection_pick = Some(None);
                                }
                                for &(worker_id, ref worker_name) in &workers {
                                    if ui
                                        .selectable_label(
                                            my_injection_worker == Some(worker_id),
                                            worker_name.as_str(),
                                        )
                                        .clicked()
                                    {
                                        pending_injection_pick = Some(Some(worker_id));
                                    }
                                }
                            });
                        });

                        ui.separator();

                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.add(egui::TextEdit::singleline(&mut researcher_data.name));
                        });

                        ui.horizontal(|ui| {
                            ui.label("Topics:");
                            egui::ComboBox::from_id_salt(
                                ui.id().with(id).with("research_topic_mode"),
                            )
                            .selected_text(
                                if researcher_data.topic_mode.is_empty() {
                                    "Select".to_string()
                                } else {
                                    researcher_data.topic_mode.clone()
                                },
                            )
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        researcher_data.topic_mode == "Articles",
                                        "Articles",
                                    )
                                    .clicked()
                                {
                                    researcher_data.topic_mode = "Articles".to_string();
                                    researcher_data.instruction = "Generate article references connected to the message context. Prefer a mix of classic and recent pieces.".to_string();
                                }
                                if ui
                                    .selectable_label(
                                        researcher_data.topic_mode == "Movies",
                                        "Movies",
                                    )
                                    .clicked()
                                {
                                    researcher_data.topic_mode = "Movies".to_string();
                                    researcher_data.instruction = "Generate movie references connected to the message context. Prefer diverse genres and well-known titles.".to_string();
                                }
                                if ui
                                    .selectable_label(
                                        researcher_data.topic_mode == "Music",
                                        "Music",
                                    )
                                    .clicked()
                                {
                                    researcher_data.topic_mode = "Music".to_string();
                                    researcher_data.instruction = "Generate music references connected to the message context. Include artist and track or album when possible.".to_string();
                                }
                            });
                        });

                        ui.horizontal(|ui| {
                            ui.label("Instruction:");
                            ui.add(egui::TextEdit::singleline(
                                &mut researcher_data.instruction,
                            ));
                        });

                        ui.horizontal(|ui| {
                            if ui
                                .checkbox(&mut researcher_data.limit_token, "Limit Tokens:")
                                .changed()
                            {
                                if !researcher_data.limit_token {
                                    researcher_data.num_predict.clear();
                                }
                            }
                            if researcher_data.limit_token {
                                ui.label("num_predict:");
                                ui.add(
                                    egui::TextEdit::singleline(&mut researcher_data.num_predict)
                                        .desired_width(80.0),
                                );
                            }
                        });
                    });
            }

            if let Some(pick) = pending_manager_pick {
                if let NodePayload::Researcher(r) = &mut agents[idx].data.payload {
                    r.manager_node = pick;
                }
            }
            if let Some(pick) = pending_injection_pick {
                if let NodePayload::Researcher(r) = &mut agents[idx].data.payload {
                    r.worker_node = pick;
                    r.active = r.worker_node.is_some();
                }
            }
        }
        AgentNodeKind::Topic => {
            {
                let topic_data = match &mut agents[idx].data.payload {
                    NodePayload::Topic(t) => t,
                    _ => unreachable!("kind mismatch"),
                };

                ui.vertical(|ui| {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        ui.add(egui::TextEdit::singleline(&mut topic_data.name));
                    });
                    ui.separator();

                    // Topic preset + topic text (mirrors Agent Worker widgets).
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label("Topic:");
                            egui::ComboBox::from_id_salt(
                                ui.id().with(id).with("topic_analysis_mode"),
                            )
                            .selected_text(if topic_data.analysis_mode.is_empty() {
                                "Select".to_string()
                            } else {
                                topic_data.analysis_mode.clone()
                            })
                            .show_ui(ui, |ui| {
                                for &(label, sentence) in TOPIC_PRESETS {
                                    if ui
                                        .selectable_label(topic_data.analysis_mode == label, label)
                                        .clicked()
                                    {
                                        topic_data.analysis_mode = label.to_string();
                                        topic_data.topic = sentence.to_string();
                                    }
                                }
                            });
                        });

                        ui.horizontal(|ui| {
                            ui.label("Topic:");
                            ui.add(egui::TextEdit::singleline(&mut topic_data.topic));
                        });
                    });
                });
            }
        }
    }
}
