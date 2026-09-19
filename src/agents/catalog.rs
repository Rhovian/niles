use std::collections::BTreeSet;

use crate::agents;

pub(super) fn model_catalog(family: &str) -> ModelCatalog {
    ModelCatalog {
        groups: static_model_groups(family),
    }
}

fn static_model_groups(family: &str) -> Vec<ModelGroup> {
    agents::default_model_aliases(family)
        .iter()
        .map(|model| ModelGroup {
            label: (*model).to_owned(),
            models: vec![(*model).to_owned()],
        })
        .collect()
}

fn ordered_group_models(label: &str, models: BTreeSet<String>) -> Vec<String> {
    let mut models = models.into_iter().collect::<Vec<_>>();
    models.sort_by(|left, right| match (left == label, right == label) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => left.cmp(right),
    });
    models
}

#[derive(Clone)]
pub(super) struct ModelGroup {
    pub(super) label: String,
    pub(super) models: Vec<String>,
}

pub(super) struct ModelCatalog {
    groups: Vec<ModelGroup>,
}

impl ModelCatalog {
    pub(super) fn groups_with_default(
        &self,
        family: &str,
        default_model: Option<&str>,
    ) -> Vec<ModelGroup> {
        let mut groups = self.groups.clone();
        if let Some(model) = default_model {
            Self::insert_model_group(&mut groups, family, model);
        }
        groups
    }

    pub(super) fn effort_options(family: &str) -> Vec<String> {
        agents::supported_efforts(family)
            .iter()
            .map(|effort| (*effort).to_owned())
            .collect()
    }

    fn insert_model_group(groups: &mut Vec<ModelGroup>, family: &str, model: &str) {
        if groups
            .iter()
            .any(|group| group.models.iter().any(|candidate| candidate == model))
        {
            return;
        }

        let label = agents::model_group_label(family, model);
        if let Some(group) = groups.iter_mut().find(|group| group.label == label) {
            group.models.push(model.to_owned());
            group.models =
                ordered_group_models(&group.label, group.models.iter().cloned().collect());
            return;
        }

        groups.push(ModelGroup {
            label,
            models: vec![model.to_owned()],
        });
    }
}
