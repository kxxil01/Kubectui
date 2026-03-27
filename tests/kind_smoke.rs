//! Ignored smoke tests for disposable kind-backed validation.

use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use kubectui::{
    app::ResourceRef,
    k8s::{
        client::K8sClient,
        exec::DebugContainerLaunchRequest,
        helm::{fetch_release_history, rollback_release},
    },
    network_policy_analysis::analyze_resource,
    network_policy_connectivity::analyze_connectivity,
    state::ClusterSnapshot,
};

const HELM_RELEASE_NAME: &str = "kubectui-smoke";
const HELM_DEPLOYMENT_NAME: &str = "kubectui-smoke";

fn kind_context_or_skip() -> Option<String> {
    if env::var("KUBECTUI_KIND_SMOKE").as_deref() != Ok("1") {
        eprintln!("skipping kind smoke test: KUBECTUI_KIND_SMOKE is not set");
        return None;
    }
    let context = kubectl(["config", "current-context"]).ok()?;
    if !context.trim().starts_with("kind-") {
        eprintln!("skipping kind smoke test: current context is not a kind context");
        return None;
    }
    Some(context.trim().to_string())
}

fn namespace(name: &str) -> String {
    format!("kubectui-smoke-{name}")
}

fn kubectl<const N: usize>(args: [&str; N]) -> Result<String, String> {
    run("kubectl", &args, None)
}

fn kubectl_in(namespace: &str, args: &[&str]) -> Result<String, String> {
    let mut full = vec!["-n", namespace];
    full.extend_from_slice(args);
    run("kubectl", &full, None)
}

fn helm(namespace: &str, args: &[&str]) -> Result<String, String> {
    let mut full = vec!["--namespace", namespace];
    full.extend_from_slice(args);
    run("helm", &full, None)
}

fn run(binary: &str, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
    let mut command = Command::new(binary);
    command.args(args);
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to spawn {binary}: {err}"))?;
    if let Some(input) = stdin
        && let Some(mut child_stdin) = child.stdin.take()
    {
        use std::io::Write;
        child_stdin
            .write_all(input.as_bytes())
            .map_err(|err| format!("failed to write stdin for {binary}: {err}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|err| format!("failed waiting for {binary}: {err}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!(
            "{binary} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn apply(namespace: &str, yaml: &str) -> Result<(), String> {
    kubectl(["create", "namespace", namespace]).ok();
    run(
        "kubectl",
        &["apply", "-n", namespace, "-f", "-"],
        Some(yaml),
    )?;
    Ok(())
}

fn delete_namespace(namespace: &str) {
    if kubectl(["get", "namespace", namespace]).is_err() {
        return;
    }
    let _ = kubectl(["delete", "namespace", namespace, "--ignore-not-found=true"]);
    let _ = kubectl([
        "wait",
        "--for=delete",
        &format!("namespace/{namespace}"),
        "--timeout=180s",
    ]);
}

fn wait_for(namespace: &str, resource: &str, condition: &str) -> Result<(), String> {
    kubectl_in(
        namespace,
        &["wait", "--for", condition, resource, "--timeout=180s"],
    )?;
    Ok(())
}

fn assert_contains(haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "expected '{haystack}' to contain '{needle}'"
    );
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = env::temp_dir().join(format!("{prefix}-{stamp}"));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

#[tokio::test]
#[ignore]
async fn kind_smoke_rollout_pause_resume_and_undo_deployment() {
    if kind_context_or_skip().is_none() {
        return;
    }
    let namespace = namespace("rollout");
    delete_namespace(&namespace);
    apply(
        &namespace,
        r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: smoke
spec:
  replicas: 1
  selector:
    matchLabels:
      app: smoke
  template:
    metadata:
      labels:
        app: smoke
    spec:
      containers:
        - name: smoke
          image: nginx:1.25.4
          ports:
            - containerPort: 80
"#,
    )
    .expect("apply rollout fixture");
    wait_for(&namespace, "deployment/smoke", "condition=Available").expect("deployment ready");

    let client = K8sClient::connect().await.expect("client");
    kubectl_in(
        &namespace,
        &["set", "image", "deployment/smoke", "smoke=nginx:1.26.3"],
    )
    .expect("upgrade deployment image");
    wait_for(&namespace, "deployment/smoke", "condition=Available").expect("deployment upgraded");

    let resource = ResourceRef::Deployment("smoke".into(), namespace.clone());
    let inspection = client
        .fetch_rollout_inspection(&resource)
        .await
        .expect("rollout inspection");
    assert!(
        inspection.revisions.len() >= 2,
        "expected at least two rollout revisions"
    );
    client
        .set_deployment_rollout_paused("smoke", &namespace, true)
        .await
        .expect("pause deployment");
    let paused = kubectl_in(
        &namespace,
        &[
            "get",
            "deployment",
            "smoke",
            "-o",
            "jsonpath={.spec.paused}",
        ],
    )
    .expect("read paused state");
    assert_eq!(paused, "true");

    client
        .set_deployment_rollout_paused("smoke", &namespace, false)
        .await
        .expect("resume deployment");
    let resumed = kubectl_in(
        &namespace,
        &[
            "get",
            "deployment",
            "smoke",
            "-o",
            "jsonpath={.spec.paused}",
        ],
    )
    .expect("read resumed state");
    assert_eq!(resumed, "false");

    client
        .rollback_workload_to_revision(&resource, 1)
        .await
        .expect("rollback deployment");
    wait_for(&namespace, "deployment/smoke", "condition=Available").expect("rollback ready");
    let image = kubectl_in(
        &namespace,
        &[
            "get",
            "deployment",
            "smoke",
            "-o",
            "jsonpath={.spec.template.spec.containers[0].image}",
        ],
    )
    .expect("read deployment image");
    assert_eq!(image, "nginx:1.25.4");
    delete_namespace(&namespace);
}

#[tokio::test]
#[ignore]
async fn kind_smoke_launch_debug_container_on_running_pod() {
    if kind_context_or_skip().is_none() {
        return;
    }
    let namespace = namespace("debug");
    delete_namespace(&namespace);
    apply(
        &namespace,
        r#"
apiVersion: v1
kind: Pod
metadata:
  name: smoke
  labels:
    app: smoke
spec:
  containers:
    - name: app
      image: busybox:1.36
      command: ["sh", "-c", "sleep 3600"]
"#,
    )
    .expect("apply debug fixture");
    wait_for(&namespace, "pod/smoke", "condition=Ready").expect("pod ready");

    let client = K8sClient::connect().await.expect("client");
    let result = client
        .launch_debug_container(&DebugContainerLaunchRequest {
            pod_name: "smoke".into(),
            namespace: namespace.clone(),
            image: "busybox:1.36".into(),
            target_container_name: Some("app".into()),
        })
        .await
        .expect("launch debug container");
    let names = kubectl_in(
        &namespace,
        &[
            "get",
            "pod",
            "smoke",
            "-o",
            "jsonpath={.spec.ephemeralContainers[*].name}",
        ],
    )
    .expect("read ephemeral container names");
    assert_contains(&names, &result.container_name);
    delete_namespace(&namespace);
}

#[tokio::test]
#[ignore]
async fn kind_smoke_helm_history_and_rollback() {
    if kind_context_or_skip().is_none() {
        return;
    }
    if env::var("KUBECTUI_SKIP_HELM_SMOKE").as_deref() == Ok("1") {
        eprintln!("skipping helm smoke test: KUBECTUI_SKIP_HELM_SMOKE=1");
        return;
    }
    let namespace = namespace("helm");
    delete_namespace(&namespace);
    kubectl(["create", "namespace", &namespace]).expect("create namespace");

    let chart_dir = unique_temp_dir("kubectui-helm-smoke");
    let chart_path = chart_dir.join("chart");
    run(
        "helm",
        &["create", chart_path.to_str().expect("chart path")],
        None,
    )
    .expect("helm create");

    let chart_arg = chart_path.to_str().expect("chart arg").to_string();
    let chart_ref = chart_arg.as_str();
    helm(
        &namespace,
        &[
            "upgrade",
            "--install",
            HELM_RELEASE_NAME,
            chart_ref,
            "--set",
            "fullnameOverride=kubectui-smoke",
            "--set",
            "image.repository=nginx",
            "--set",
            "image.tag=1.25.4",
            "--wait",
        ],
    )
    .expect("helm install");
    helm(
        &namespace,
        &[
            "upgrade",
            HELM_RELEASE_NAME,
            chart_ref,
            "--set",
            "fullnameOverride=kubectui-smoke",
            "--set",
            "image.repository=nginx",
            "--set",
            "image.tag=1.26.3",
            "--wait",
        ],
    )
    .expect("helm upgrade");

    let history = fetch_release_history(HELM_RELEASE_NAME, &namespace, None)
        .await
        .expect("fetch helm history");
    assert!(
        history.revisions.len() >= 2,
        "expected at least two helm revisions"
    );
    rollback_release(HELM_RELEASE_NAME, &namespace, None, 1)
        .await
        .expect("helm rollback");
    let image = kubectl_in(
        &namespace,
        &[
            "get",
            "deployment",
            HELM_DEPLOYMENT_NAME,
            "-o",
            "jsonpath={.spec.template.spec.containers[0].image}",
        ],
    )
    .expect("read helm deployment image");
    assert_eq!(image, "nginx:1.25.4");

    let _ = fs::remove_dir_all(&chart_dir);
    delete_namespace(&namespace);
}

#[tokio::test]
#[ignore]
async fn kind_smoke_network_policy_analysis_and_connectivity() {
    if kind_context_or_skip().is_none() {
        return;
    }
    let namespace = namespace("netpol");
    delete_namespace(&namespace);
    apply(
        &namespace,
        r#"
apiVersion: v1
kind: Pod
metadata:
  name: client
  labels:
    role: client
spec:
  containers:
    - name: app
      image: busybox:1.36
      command: ["sh", "-c", "sleep 3600"]
---
apiVersion: v1
kind: Pod
metadata:
  name: blocked
  labels:
    role: blocked
spec:
  containers:
    - name: app
      image: busybox:1.36
      command: ["sh", "-c", "sleep 3600"]
---
apiVersion: v1
kind: Pod
metadata:
  name: server
  labels:
    role: server
spec:
  containers:
    - name: app
      image: busybox:1.36
      command: ["sh", "-c", "sleep 3600"]
---
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: allow-client
spec:
  podSelector:
    matchLabels:
      role: server
  policyTypes:
    - Ingress
  ingress:
    - from:
        - podSelector:
            matchLabels:
              role: client
"#,
    )
    .expect("apply network policy fixture");
    wait_for(&namespace, "pod/client", "condition=Ready").expect("client ready");
    wait_for(&namespace, "pod/blocked", "condition=Ready").expect("blocked ready");
    wait_for(&namespace, "pod/server", "condition=Ready").expect("server ready");

    let client = K8sClient::connect().await.expect("client");
    let mut snapshot = ClusterSnapshot {
        pods: client.fetch_pods(Some(&namespace)).await.expect("pods"),
        network_policies: client
            .fetch_network_policies(Some(&namespace))
            .await
            .expect("network policies"),
        namespace_list: vec![kubectui::k8s::dtos::NamespaceInfo {
            name: namespace.clone(),
            status: "Active".into(),
            ..Default::default()
        }],
        ..ClusterSnapshot::default()
    };
    snapshot.namespaces_count = snapshot.namespace_list.len();

    let analysis = analyze_resource(
        &ResourceRef::Pod("server".into(), namespace.clone()),
        &snapshot,
    )
    .expect("network policy analysis");
    assert!(
        analysis
            .summary_lines
            .iter()
            .any(|line| line.contains("selected by 1 NetworkPolicy")),
        "expected selected-by-policy summary"
    );

    let allowed = analyze_connectivity(
        &ResourceRef::Pod("client".into(), namespace.clone()),
        &ResourceRef::Pod("server".into(), namespace.clone()),
        &snapshot,
    )
    .expect("allowed connectivity");
    assert!(
        allowed.summary_lines[0].contains("ALLOW"),
        "expected allow verdict"
    );

    let denied = analyze_connectivity(
        &ResourceRef::Pod("blocked".into(), namespace.clone()),
        &ResourceRef::Pod("server".into(), namespace.clone()),
        &snapshot,
    )
    .expect("denied connectivity");
    assert!(
        denied.summary_lines[0].contains("DENY"),
        "expected deny verdict"
    );

    delete_namespace(&namespace);
}
