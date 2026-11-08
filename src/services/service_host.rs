//! Port of Service / ServiceCollection / ServiceHost.
//! Services are fields of [`App`]; a descriptor carries each service's lifecycle hooks.

use crate::app::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ServiceHostLifecycle {
    #[default]
    NotInitialized,
    PreInitialized,
    Initialized,
    PostInitialized,
    Terminated,
}

pub type Hook = fn(&mut App);

#[derive(Clone)]
pub struct ServiceDescriptor {
    pub name: &'static str,
    pub dependencies: &'static [&'static str],
    pub on_pre_initialized: Hook,
    pub on_initialized: Hook,
    pub on_post_initialized: Hook,
    pub on_terminated: Hook,
}

pub fn noop(_: &mut App) {}

/// Dependency ordered services (a dependency is always placed before its dependents)
pub fn dependency_ordered(services: &[ServiceDescriptor]) -> Result<Vec<ServiceDescriptor>, String> {
    let mut seen = std::collections::HashSet::new();
    for s in services {
        if !seen.insert(s.name) {
            return Err(format!("Service with name '{}' already exists and cannot be used", s.name));
        }
    }
    let find = |name: &str| services.iter().find(|s| s.name == name);
    let mut sorted: Vec<ServiceDescriptor> = Vec::new();
    let mut visited = std::collections::HashSet::new();

    fn visit<'a>(
        service: &'a ServiceDescriptor,
        stack: &mut Vec<&'static str>,
        visited: &mut std::collections::HashSet<&'static str>,
        sorted: &mut Vec<ServiceDescriptor>,
        find: &dyn Fn(&str) -> Option<&'a ServiceDescriptor>,
    ) -> Result<(), String> {
        if visited.contains(service.name) {
            return Ok(());
        }
        visited.insert(service.name);
        stack.push(service.name);
        for dep in service.dependencies {
            let Some(dep_service) = find(dep) else {
                return Err(format!("Service '{}' depends on '{}' but it is not in the service collection", service.name, dep));
            };
            if stack.contains(dep) {
                return Err(format!("Service '{}' depends on '{}' but this would result in a circular dependency", service.name, dep));
            }
            visit(dep_service, stack, visited, sorted, find)?;
        }
        stack.pop();
        sorted.push(service.clone());
        Ok(())
    }

    for s in services {
        if !visited.contains(s.name) {
            let mut stack = Vec::new();
            visit(s, &mut stack, &mut visited, &mut sorted, &find)?;
        }
    }
    Ok(sorted)
}

#[derive(Default)]
pub struct ServiceHost {
    pub lifecycle: ServiceHostLifecycle,
    services: Vec<ServiceDescriptor>,
}

impl ServiceHost {
    pub fn initialized(&self) -> bool {
        matches!(self.lifecycle, ServiceHostLifecycle::Initialized | ServiceHostLifecycle::PostInitialized)
    }

    pub fn set_services(&mut self, services: Vec<ServiceDescriptor>) {
        self.services = dependency_ordered(&services).expect("invalid service collection");
    }
}

/// Executes the next lifecycle for the service host
pub fn run_next_lifecycle(app: &mut App) {
    let next = match app.host.lifecycle {
        ServiceHostLifecycle::NotInitialized => ServiceHostLifecycle::PreInitialized,
        ServiceHostLifecycle::PreInitialized => ServiceHostLifecycle::Initialized,
        ServiceHostLifecycle::Initialized => ServiceHostLifecycle::PostInitialized,
        ServiceHostLifecycle::PostInitialized => ServiceHostLifecycle::Terminated,
        ServiceHostLifecycle::Terminated => return,
    };
    app.host.lifecycle = next;
    let services = app.host.services.clone();
    for s in services {
        let hook = match next {
            ServiceHostLifecycle::PreInitialized => s.on_pre_initialized,
            ServiceHostLifecycle::Initialized => s.on_initialized,
            ServiceHostLifecycle::PostInitialized => s.on_post_initialized,
            ServiceHostLifecycle::Terminated => s.on_terminated,
            ServiceHostLifecycle::NotInitialized => continue,
        };
        hook(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(name: &'static str, deps: &'static [&'static str]) -> ServiceDescriptor {
        ServiceDescriptor { name, dependencies: deps, on_pre_initialized: noop, on_initialized: noop, on_post_initialized: noop, on_terminated: noop }
    }

    #[test]
    fn orders_dependencies_first() {
        let order = dependency_ordered(&[d("A", &[]), d("B", &[]), d("C", &["D"]), d("D", &[])]).unwrap();
        let names: Vec<_> = order.iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["A", "B", "D", "C"]);
    }

    #[test]
    fn detects_errors() {
        assert!(dependency_ordered(&[d("A", &["B"]), d("B", &["A"])]).err().unwrap().contains("circular"));
        assert!(dependency_ordered(&[d("A", &["X"])]).err().unwrap().contains("not in the service collection"));
        assert!(dependency_ordered(&[d("A", &[]), d("A", &[])]).err().unwrap().contains("already exists"));
    }
}
