use crate::error::{OlmaError, Result};
use crate::metadata::{Formula, client::{FetchPolicy, FormulaeClient}};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub ordered: Vec<Formula>,
    pub requested: HashSet<String>,
}

pub struct Resolver<'a> {
    client: &'a FormulaeClient,
    policy: FetchPolicy,
}

impl<'a> Resolver<'a> {
    pub fn new(client: &'a FormulaeClient, policy: FetchPolicy) -> Self {
        Self { client, policy }
    }

    pub async fn resolve(&self, targets: &[String]) -> Result<InstallPlan> {
        let requested: HashSet<String> = targets.iter().cloned().collect();
        let mut graph: HashMap<String, Formula> = HashMap::new();
        let mut deps: HashMap<String, Vec<String>> = HashMap::new();
        let mut queue: VecDeque<String> = targets.iter().cloned().collect();

        while let Some(name) = queue.pop_front() {
            if graph.contains_key(&name) {
                continue;
            }
            let formula = self.client.fetch(&name, self.policy).await?;
            let dep_names = formula.dependencies.clone();
            for d in &dep_names {
                if !graph.contains_key(d) {
                    queue.push_back(d.clone());
                }
            }
            deps.insert(name.clone(), dep_names);
            graph.insert(name, formula);
        }

        let ordered_names = Self::topo_sort(&deps)?;
        let ordered: Vec<Formula> = ordered_names.into_iter()
            .filter_map(|n| graph.remove(&n))
            .collect();

        Ok(InstallPlan { ordered, requested })
    }

    fn topo_sort(deps: &HashMap<String, Vec<String>>) -> Result<Vec<String>> {
        let mut indegree: HashMap<String, usize> = HashMap::new();
        for node in deps.keys() {
            indegree.entry(node.clone()).or_insert(0);
        }
        for (_node, ds) in deps {
            for d in ds {
                *indegree.entry(d.clone()).or_insert(0) += 1;
            }
        }

        let mut ready: VecDeque<String> = indegree.iter()
            .filter(|(_, d)| **d == 0)
            .map(|(n, _)| n.clone())
            .collect();

        let mut out: Vec<String> = Vec::new();
        while let Some(node) = ready.pop_front() {
            if let Some(children) = deps.get(&node) {
                for c in children {
                    if let Some(d) = indegree.get_mut(c) {
                        *d -= 1;
                        if *d == 0 {
                            ready.push_back(c.clone());
                        }
                    }
                }
            }
            out.push(node);
        }

        if out.len() != indegree.len() {
            return Err(OlmaError::Other("dependency cycle detected".into()));
        }
        out.reverse();
        Ok(out)
    }

    pub fn topo_sort_for_test(deps: &HashMap<String, Vec<String>>) -> Result<Vec<String>> {
        Self::topo_sort(deps)
    }
}
