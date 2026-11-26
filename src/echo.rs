//! DTN Postmaster Echo Service with automatic traceback return-path routing.
//!
//! Mirrors `draft-taylor-dtn-echo-service` behavior while deriving a reversed
//! strict APRS `TrajectoryControl` (Block Type 200) from incoming TREB (Block
//! Type 201), completed APRS hops, or Previous Node (Block Type 6) so echo
//! replies traverse the exact reverse path even on route-free networks.

use dtnpm_core::mesh_blocks::{apply_reply_routing, derive_reply_routing};
use hardy_async::sync::spin::Once;
use hardy_bpa::{
    async_trait,
    services::{Service, ServiceSink, StatusNotify},
    stream::{Receiver, Segment, buffer_stream},
};
use hardy_bpv7::{
    builder::Builder,
    bundle::{Flags, Id},
    creation_timestamp::CreationTimestamp,
    eid::Eid,
    parse::{Parsed, parse},
    status_report::ReasonCode,
};
use tracing::{debug, warn};

/// Built-in DTN Postmaster echo service supporting automatic traceback replies.
pub struct DtnpmEchoService {
    local_eid: Once<Eid>,
    sink: Once<Box<dyn ServiceSink>>,
}

impl Default for DtnpmEchoService {
    fn default() -> Self {
        Self::new()
    }
}

impl DtnpmEchoService {
    /// Creates a new [`DtnpmEchoService`] with no BPA sink attached yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            local_eid: Once::new(),
            sink: Once::new(),
        }
    }

    /// Unregisters the echo service from the BPA if registered.
    pub async fn unregister(&self) {
        if let Some(sink) = self.sink.get() {
            sink.unregister().await;
        }
    }

    fn build_response(&self, data: hardy_bpa::Bytes) -> Option<(Eid, Eid, Vec<u8>)> {
        let Parsed { data, bundle, .. } = parse(data)
            .inspect_err(|e| debug!("Failed to parse incoming bundle: {e:?}"))
            .ok()?;

        if bundle.primary.id.source.is_null() {
            debug!("Not echoing bundle from the null endpoint");
            return None;
        }
        if bundle.primary.flags.is_admin_record {
            debug!("Not echoing administrative record");
            return None;
        }

        let Some(payload) = bundle.blocks.get(&1).and_then(|block| block.payload(&data)) else {
            debug!("Incoming bundle has no payload block");
            return None;
        };

        let source_eid = self
            .local_eid
            .get()
            .cloned()
            .unwrap_or_else(|| bundle.primary.destination.clone());
        let dest_eid = bundle.primary.id.source.clone();

        debug!(
            source = %source_eid,
            destination = %dest_eid,
            "Received bundle, building echo response"
        );

        let mut builder = Builder::new(source_eid.clone(), dest_eid.clone())
            .with_lifetime(bundle.primary.lifetime)
            .with_flags(response_flags(&bundle.primary.flags));

        if requested_status_reports(&bundle.primary.flags) {
            builder = builder.with_report_to(bundle.primary.report_to.clone());
        }

        let reply_routing = derive_reply_routing(&bundle, &data, &source_eid);
        if let Some(ref traj) = reply_routing.return_trajectory {
            debug!(
                hops = traj.path_elements.len(),
                destination = %dest_eid,
                "Attaching traceback return trajectory to echo response"
            );
        }
        builder = match apply_reply_routing(builder, &reply_routing) {
            Ok(b) => b,
            Err(e) => {
                warn!("Failed to attach reply routing to echo response: {e}");
                return None;
            }
        };

        let (_, response) = builder
            .with_payload(payload.into())
            .build(CreationTimestamp::now())
            .inspect_err(|e| debug!("Failed to build echo response: {e:?}"))
            .ok()?;

        Some((source_eid, dest_eid, response.into_vec()))
    }

    async fn echo(&self, data: hardy_bpa::Bytes) -> Result<(), hardy_bpa::Error> {
        let Some(sink) = self.sink.get() else {
            return Ok(());
        };
        let Some((source_eid, dest_eid, response)) = self.build_response(data) else {
            return Ok(());
        };

        debug!(
            source = %source_eid,
            destination = %dest_eid,
            "Sending echo response"
        );

        sink.send(&mut hardy_bpa::Bytes::from(response))
            .await
            .inspect_err(|e| {
                warn!("Failed to send echo response: {e:?}");
            })?;

        Ok(())
    }
}

#[async_trait]
impl Service for DtnpmEchoService {
    async fn on_register(&self, source: &Eid, sink: Box<dyn ServiceSink>) {
        self.local_eid.call_once(|| source.clone());
        self.sink.call_once(|| sink);
    }

    async fn on_unregister(&self) {}

    async fn on_status_notify(
        &self,
        _bundle_id: &Id,
        _from: &Eid,
        _kind: StatusNotify,
        _reason: ReasonCode,
        _timestamp: Option<time::OffsetDateTime>,
    ) {
    }

    async fn on_deliver(
        &self,
        _bundle_id: &Id,
        _expiry: time::OffsetDateTime,
        total_len: u64,
        stream: &mut dyn Receiver<Segment>,
    ) -> hardy_bpa::services::Result<()> {
        let data = buffer_stream(stream, total_len).await?;
        self.echo(data).await.map_err(Into::into)
    }
}

fn requested_status_reports(flags: &Flags) -> bool {
    flags.receipt_report_requested
        || flags.forward_report_requested
        || flags.delivery_report_requested
        || flags.delete_report_requested
}

fn response_flags(request: &Flags) -> Flags {
    let mut flags = Flags {
        do_not_fragment: request.do_not_fragment,
        ..Default::default()
    };
    if requested_status_reports(request) {
        flags.report_status_time = request.report_status_time;
        flags.receipt_report_requested = request.receipt_report_requested;
        flags.forward_report_requested = request.forward_report_requested;
        flags.delivery_report_requested = request.delivery_report_requested;
        flags.delete_report_requested = request.delete_report_requested;
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;
    use dtnpm_core::mesh_blocks::{
        DEFAULT_BLOCK_TYPE_TREB, TrebData, TrebHopEntry, extract_routing_extensions,
        status_report_flags,
    };
    use hardy_bpv7::block;

    #[test]
    fn echo_response_reverses_treb_and_resets_reply_treb() {
        let src: Eid = "dtn://stata/dtnping-1".parse().expect("src");
        let dst: Eid = "dtn://statd/echo".parse().expect("dst");
        let treb = TrebData {
            hops: vec![
                TrebHopEntry {
                    node_eid: "dtn://stata/".to_string(),
                    arrival_time_ms: Some(1),
                    departure_time_ms: Some(2),
                    cla_info: None,
                },
                TrebHopEntry {
                    node_eid: "dtn://statb/".to_string(),
                    arrival_time_ms: Some(3),
                    departure_time_ms: Some(4),
                    cla_info: None,
                },
                TrebHopEntry {
                    node_eid: "dtn://statc/".to_string(),
                    arrival_time_ms: Some(5),
                    departure_time_ms: Some(6),
                    cla_info: None,
                },
            ],
        };
        let mut req_builder = Builder::new(src.clone(), dst.clone())
            .with_payload(b"ping-payload".as_slice().into())
            .with_flags(status_report_flags());
        req_builder = req_builder
            .add_extension_block(block::Type::Unrecognised(DEFAULT_BLOCK_TYPE_TREB))
            .expect("add treb")
            .build(treb.encode_cbor().expect("encode").into_vec().into());
        let (_req_b, req_raw) = req_builder.build(CreationTimestamp::now()).expect("build");

        let svc = DtnpmEchoService::new();
        svc.local_eid.call_once(|| dst.clone());
        let (resp_src, resp_dst, resp_raw) = svc
            .build_response(hardy_bpa::Bytes::from(req_raw.into_vec()))
            .expect("echo response must be built");
        assert_eq!(resp_src, dst);
        assert_eq!(resp_dst, src);

        let parsed = parse(hardy_bpa::Bytes::from(resp_raw)).expect("parse response");
        let ext = extract_routing_extensions(&parsed.bundle, &parsed.data);
        assert_eq!(
            ext.aprs_path_formatted.as_deref(),
            Some("dtn://statc/,dtn://statb/")
        );
        assert_eq!(ext.treb, Some(TrebData::default()));
        assert_eq!(parsed.bundle.primary.flags, status_report_flags());
    }
}
