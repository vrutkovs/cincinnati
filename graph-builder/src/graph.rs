// Copyright 2018 Alex Crawford
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
use crate::built_info;
use crate::config;
use actix_web::{HttpRequest, HttpResponse};
use cincinnati::plugins::prelude::*;
use cincinnati::CONTENT_TYPE;
use commons::metrics::HasRegistry;
use commons::GraphError;
use failure::Fallible;
use lazy_static;
pub use parking_lot::RwLock;
use prometheus::{self, histogram_opts, labels, opts, Counter, Gauge, Histogram, IntGauge};
use rustracing::tag::Tag;
use rustracing_jaeger::span::SpanContext;
use rustracing_jaeger::Tracer;
use serde_json;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::thread;

lazy_static! {
    static ref GRAPH_FINAL_RELEASES: IntGauge = IntGauge::new(
        "graph_final_releases",
        "Number of releases in the final graph, after processing"
    )
    .unwrap();
    static ref GRAPH_LAST_SUCCESSFUL_REFRESH: IntGauge = IntGauge::new(
        "graph_last_successful_refresh_timestamp",
        "UTC timestamp of last successful graph refresh"
    )
    .unwrap();
    static ref UPSTREAM_ERRORS: Counter = Counter::new(
        "graph_upstream_errors_total",
        "Total number of upstream scraping errors"
    )
    .unwrap();
    static ref UPSTREAM_SCRAPES: Counter = Counter::new(
        "graph_upstream_scrapes_total",
        "Total number of upstream scrapes"
    )
    .unwrap();
    static ref GRAPH_UPSTREAM_INITIAL_SCRAPE: Gauge = Gauge::new(
        "graph_initial_upstream_scrape_duration",
        "Duration of initial upstream scrape"
    )
    .unwrap();
    /// Histogram with custom bucket values for upstream scraping duration in seconds
    static ref UPSTREAM_SCRAPES_DURATION: Histogram = Histogram::with_opts(histogram_opts!(
        "graph_upstream_scrapes_duration",
        "Upstream scrape duration in seconds",
        vec![5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 15.0, 20.0, 30.0 ]
    ))
    .unwrap();
    static ref V1_GRAPH_INCOMING_REQS: Counter = Counter::new(
        "v1_graph_incoming_requests_total",
        "Total number of incoming HTTP client request to /v1/graph"
    )
    .unwrap();
    static ref BUILD_INFO: Counter = Counter::with_opts(opts!(
        "build_info",
        "Build information",
        labels!{
            "git_commit" => match built_info::GIT_VERSION {
                Some(commit) => commit,
                None => "unknown"
            },
        }
    ))
    .unwrap();
}

/// Register relevant metrics to a prometheus registry.
pub fn register_metrics(registry: &prometheus::Registry) -> Fallible<()> {
    commons::register_metrics(&registry)?;
    registry.register(Box::new(GRAPH_FINAL_RELEASES.clone()))?;
    registry.register(Box::new(GRAPH_LAST_SUCCESSFUL_REFRESH.clone()))?;
    registry.register(Box::new(UPSTREAM_ERRORS.clone()))?;
    registry.register(Box::new(UPSTREAM_SCRAPES.clone()))?;
    registry.register(Box::new(GRAPH_UPSTREAM_INITIAL_SCRAPE.clone()))?;
    registry.register(Box::new(UPSTREAM_SCRAPES_DURATION.clone()))?;
    registry.register(Box::new(V1_GRAPH_INCOMING_REQS.clone()))?;
    registry.register(Box::new(BUILD_INFO.clone()))?;
    Ok(())
}

/// Serve Cincinnati graph requests.
pub async fn index(
    req: HttpRequest,
    app_data: actix_web::web::Data<State>,
) -> Result<HttpResponse, GraphError> {
    let mut carrier: HashMap<String, String> = HashMap::new();
    let headers = req.headers();
    let query_string = req.query_string().to_string();
    for (k, v) in headers {
        carrier.insert(k.to_string(), v.to_str().unwrap().to_string());
    }

    let context = track_try_unwrap!(SpanContext::extract_from_http_header(&carrier));
    let mut _span = app_data.get_ref().tracer.span("index").child_of(&context);
    for (k, v) in carrier {
        _span = _span.tag(Tag::new(k, v))
    }
    // Tracing: keep query_string as a tag
    _span = _span.tag(Tag::new("query_string", query_string));
    let _ = _span.start();

    V1_GRAPH_INCOMING_REQS.inc();

    // Check that the client can accept JSON media type.
    commons::ensure_content_type(req.headers(), CONTENT_TYPE)?;

    // Check for required client parameters.
    let mandatory_params = &app_data.mandatory_params;
    commons::ensure_query_params(mandatory_params, req.query_string())?;

    let resp = HttpResponse::Ok()
        .content_type(CONTENT_TYPE)
        .body(app_data.json.read().clone());
    Ok(resp)
}

#[derive(Clone)]
pub struct State {
    json: Arc<RwLock<String>>,
    /// Query parameters that must be present in all client requests.
    mandatory_params: HashSet<String>,
    live: Arc<RwLock<bool>>,
    ready: Arc<RwLock<bool>>,
    plugins: &'static [BoxedPlugin],
    registry: &'static prometheus::Registry,
    tracer: Tracer,
}

impl State {
    /// Creates a new State with the given arguments
    pub fn new(
        json: Arc<RwLock<String>>,
        mandatory_params: HashSet<String>,
        live: Arc<RwLock<bool>>,
        ready: Arc<RwLock<bool>>,
        plugins: &'static [BoxedPlugin],
        registry: &'static prometheus::Registry,
        tracer: Tracer,
    ) -> State {
        State {
            json,
            mandatory_params,
            live,
            ready,
            plugins,
            registry,
            tracer,
        }
    }

    /// Returns the boolean inside self.live
    pub fn is_live(&self) -> bool {
        *self.live.read()
    }

    /// Returns the boolean inside self.ready
    pub fn is_ready(&self) -> bool {
        *self.ready.read()
    }
}

impl HasRegistry for State {
    fn registry(&self) -> &'static prometheus::Registry {
        self.registry
    }
}

#[allow(clippy::useless_let_if_seq)]
pub async fn run(settings: &config::AppSettings, state: &State) -> ! {
    // Indicate if a panic happens
    let previous_hook = std::panic::take_hook();
    let panic_live = state.live.clone();
    std::panic::set_hook(Box::new(move |panic_info| {
        *panic_live.write() = false;
        previous_hook(panic_info)
    }));

    // Don't wait on the first iteration
    let mut first_iteration = true;
    let mut first_success = true;

    BUILD_INFO.inc();

    loop {
        // Store scrape duration value. It would be used for initial scrape gauge or scrape histogram
        let scrape_value: f64;

        if first_iteration {
            *state.live.write() = true;
            first_iteration = false;
        } else {
            thread::sleep(settings.pause_secs);
        }

        debug!("graph update triggered");
        let scrape_timer = UPSTREAM_SCRAPES_DURATION.start_timer();

        let carrier: HashMap<String, String> = HashMap::new();

        let context = track_try_unwrap!(SpanContext::extract_from_http_header(&carrier));
        let mut span = state.tracer.span("scrape").child_of(&context).start();

        let scrape = cincinnati::plugins::process(
            state.plugins.iter(),
            cincinnati::plugins::PluginIO::InternalIO(cincinnati::plugins::InternalIO {
                // the first plugin will produce the initial graph
                graph: Default::default(),
                // the plugins used in the graph-builder don't expect any parameters yet
                parameters: Default::default(),
            }),
            &span,
            &state.tracer,
        )
        .await;
        UPSTREAM_SCRAPES.inc();

        span.log(|log| {
            log.std().message("plugins processed");
        });

        let internal_io = match scrape {
            Ok(internal_io) => internal_io,
            Err(err) => {
                UPSTREAM_ERRORS.inc();
                err.iter_chain().for_each(|cause| error!("{}", cause));
                continue;
            }
        };

        let json_graph = match serde_json::to_string(&internal_io.graph) {
            Ok(json) => json,
            Err(err) => {
                UPSTREAM_ERRORS.inc();
                error!("Failed to serialize graph: {}", err);
                continue;
            }
        };
        span.log(|log| {
            log.std().message("json marshalled");
        });

        *state.json.write() = json_graph;

        span.log(|log| {
            log.std().message("state written");
        });

        // Record scrape duration
        scrape_value = scrape_timer.stop_and_discard();

        if first_success {
            *state.ready.write() = true;
            first_success = false;
            GRAPH_UPSTREAM_INITIAL_SCRAPE.set(scrape_value);
        } else {
            UPSTREAM_SCRAPES_DURATION.observe(scrape_value);
        }

        GRAPH_LAST_SUCCESSFUL_REFRESH.set(chrono::Utc::now().timestamp() as i64);

        let nodes_count = internal_io.graph.releases_count();
        GRAPH_FINAL_RELEASES.set(nodes_count as i64);
        debug!("graph update completed, {} valid releases", nodes_count);
        span.log(|log| {
            log.std().message("done");
        });
    }
}
