//! Cross-crate pull e2e (plan T07): a real control plane (via
//! `control_plane::main` on a dedicated thread) plus the real worker poll,
//! reconcile, and heartbeat pieces over loopback RPC, with fake VM execution.
//!
//! No VMs boot for real (no KVM here); the fake factory stands in at the
//! execution edge exactly like the unit tests. What this proves end to end:
//! publish → poll → reconcile → heartbeat → cluster status converges, then
//! removal → delete → converged again — with no manual worker RPC anywhere.
//!
//! The loop tasks themselves are driven piece by piece (deterministic, no
//! sleeps); `poll_loop`/`heartbeat_loop` timing is covered by their own tests.

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use capnp::message::ReaderOptions;
    use capnp_rpc::{RpcSystem, rpc_twoparty_capnp, twoparty};
    use futures::AsyncReadExt;
    use tokio::sync::mpsc;
    use tokio_util::compat::TokioAsyncReadCompatExt;

    use crate::{
        heartbeat::{PushClient, build_report},
        pull::{AppliedGeneration, PullClient, Reconciler, RpcPullClient},
        test_support::{FakeFactory, TempDb},
        vmm::{Command, Registry, Writer},
    };
    use control_plane::registry::{DeclaredWorker, WorkerId};

    type ChMasterClient =
        commands::master_capnp::master::Client<commands::ch_capnp::vm_config::Owned>;

    /// Boots a real control plane on an ephemeral port in its own thread
    /// (keeps runtimes un-nested) and returns its address.
    fn spawn_control_plane() -> std::net::SocketAddr {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("probe bind");
        let addr: std::net::SocketAddr = probe.local_addr().expect("probe addr");
        drop(probe);

        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("control plane runtime")
                .block_on(control_plane::main::<capnp::any_pointer::Owned>(
                    "cp-e2e".to_string(),
                    addr,
                    Vec::new(),
                    vec![DeclaredWorker::new(WorkerId::new("w1").expect("worker id"))],
                ));
        });
        addr
    }

    async fn connect_master(addr: std::net::SocketAddr) -> ChMasterClient {
        let stream = tokio::net::TcpStream::connect(addr)
            .await
            .expect("master dial");
        stream.set_nodelay(true).expect("nodelay");
        let (reader, writer) = TokioAsyncReadCompatExt::compat(stream).split();
        let network = twoparty::VatNetwork::new(
            futures::io::BufReader::new(reader),
            futures::io::BufWriter::new(writer),
            rpc_twoparty_capnp::Side::Client,
            ReaderOptions::default(),
        );
        let mut rpc_system = RpcSystem::new(Box::new(network), None);
        let client: ChMasterClient = rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);
        tokio::task::spawn_local(rpc_system);
        client
    }

    async fn publish(
        client: &ChMasterClient,
        commit: &str,
        generation: u64,
        specs: &[(&str, &str, &str)],
    ) {
        let mut request = client.publish_state_request();
        {
            let mut params = request.get();
            params.set_commit(commit);
            params.set_generation(generation);
            params.set_intent_hash("intent-e2e");
            let count = u32::try_from(specs.len()).unwrap_or(u32::MAX);
            let mut wire = params.reborrow().init_vm_specs(count);
            for (index, (id, target, kernel)) in specs.iter().enumerate() {
                let index = u32::try_from(index).unwrap_or(u32::MAX);
                let mut slot = wire.reborrow().get(index);
                slot.set_id(id);
                slot.set_target_worker(target);
                let mut payload = slot.reborrow().init_spec();
                let mut config = payload.reborrow().init_payload();
                config.set_kernel(kernel);
                config.set_cmdline("console=ttyS0");
                config.set_initramfs("initrd");
            }
        }
        let response = request.send().promise.await.expect("publish rpc");
        let result = response
            .get()
            .expect("publish response")
            .get_result()
            .expect("publish result");
        match result.which().expect("publish union") {
            commands::common_capnp::result::Which::Ok(_) => {}
            commands::common_capnp::result::Which::Err(text) => {
                panic!(
                    "publish rejected: {}",
                    text.expect("err").to_str().expect("text")
                );
            }
        }
    }

    async fn cluster_status(client: &ChMasterClient) -> (u64, String, u32, usize) {
        let request = client.get_cluster_status_request();
        let response = request.send().promise.await.expect("status rpc");
        let status = response
            .get()
            .expect("status response")
            .get_status()
            .expect("status body");
        (
            status.get_active_generation(),
            status
                .get_active_commit()
                .expect("commit")
                .to_str()
                .expect("commit text")
                .to_owned(),
            status.get_convergence_percent(),
            status.get_workers().expect("workers").len() as usize,
        )
    }

    /// Applies reconciler commands the way the supervisor would (test double
    /// for the execution edge only; routing all comes from the real pull).
    async fn apply_all(
        rx: &mut mpsc::Receiver<Command<FakeFactory>>,
        writer: &mut Registry<FakeFactory, Writer>,
    ) {
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Command::Create(cmd) => {
                    let (handle, id) = cmd.into_parts();
                    writer.insert(id, handle).await;
                }
                Command::Delete(id) => {
                    writer.remove(&id).await;
                }
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pull_e2e_create_delete_converges() {
        let cp_addr = spawn_control_plane();

        tokio::task::LocalSet::new()
            .run_until(async {
                // Wait for the control plane listener (bounded retries).
                let worker_client: RpcPullClient<capnp::any_pointer::Owned> = {
                    let mut dials = 0;
                    loop {
                        match RpcPullClient::connect(cp_addr).await {
                            Ok(client) => break client,
                            Err(_) if dials < 40 => {
                                dials += 1;
                                tokio::time::sleep(Duration::from_millis(50)).await;
                            }
                            Err(err) => panic!("control plane never came up: {err}"),
                        }
                    }
                };
                let master = connect_master(cp_addr).await;

                let (db, _guard) = TempDb::create("e2e").await;
                let registry = Registry::<FakeFactory, _>::new(db);
                let (reader, mut writer) = registry.split();
                let (tx, mut rx) = mpsc::channel(16);
                let reconciler = Reconciler::new(FakeFactory, tx);
                let applied = AppliedGeneration::new();

                // Generation 1: one VM explicitly for w1, end to end.
                publish(&master, "commit-1", 1, &[("vm-e2e", "w1", "vmlinux-e2e")]).await;
                let assignment = worker_client.fetch("w1", 0).await.expect("poll");
                assert_eq!(assignment.generation, 1);
                assert_eq!(
                    assignment
                        .desired
                        .iter()
                        .map(|vm| vm.key.as_str())
                        .collect::<Vec<_>>(),
                    vec!["vm-e2e"]
                );
                let outcome = reconciler
                    .reconcile(&assignment.desired, &reader)
                    .await
                    .expect("reconcile");
                assert_eq!(outcome.created, vec!["vm-e2e".to_string()]);
                apply_all(&mut rx, &mut writer).await;
                applied.set(assignment.generation);

                let report = build_report::<FakeFactory>("w1", applied.get(), &reader).await;
                assert_eq!(report.observed_generation, 1);
                assert_eq!(report.running.len(), 1);
                PushClient::push(&worker_client, &report)
                    .await
                    .expect("heartbeat");
                assert_eq!(
                    cluster_status(&master).await,
                    (1, "commit-1".to_string(), 100, 1)
                );

                // Generation 2: removal flows the same way back to converged.
                publish(&master, "commit-2", 2, &[]).await;
                let assignment = worker_client.fetch("w1", 1).await.expect("repoll");
                assert_eq!(assignment.generation, 2);
                assert!(assignment.desired.is_empty());
                let outcome = reconciler
                    .reconcile(&assignment.desired, &reader)
                    .await
                    .expect("reconcile removal");
                assert_eq!(outcome.deleted, vec!["vm-e2e".to_string()]);
                apply_all(&mut rx, &mut writer).await;
                applied.set(assignment.generation);

                let report = build_report::<FakeFactory>("w1", applied.get(), &reader).await;
                assert_eq!(report.observed_generation, 2);
                assert!(report.running.is_empty());
                PushClient::push(&worker_client, &report)
                    .await
                    .expect("heartbeat");
                assert_eq!(
                    cluster_status(&master).await,
                    (2, "commit-2".to_string(), 100, 1)
                );
            })
            .await;
    }
}
