//! Cincinnati backend: policy-engine server.

#![deny(missing_docs)]

#[macro_use]
extern crate cincinnati;
#[macro_use]
extern crate commons;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate smart_default;
#[macro_use]
extern crate structopt;
#[macro_use]
extern crate custom_debug_derive;
#[macro_use]
extern crate trackable;

mod config;
mod graph;
mod openapi;

use actix_web::{App, HttpServer};
use cincinnati::plugins::BoxedPlugin;
use commons::metrics::{self, RegistryWrapper};
use failure::Error;
use prometheus::{labels, opts, Counter, Registry};
use std::collections::HashSet;

use rustracing_jaeger::reporter::JaegerCompactReporter;
use rustracing_jaeger::Tracer;

#[allow(dead_code)]
/// Build info
mod built_info {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

/// Common prefix for policy-engine metrics.
pub static METRICS_PREFIX: &str = "cincinnati_pe";

lazy_static! {
    static ref BUILD_INFO: Counter = Counter::with_opts(opts!(
        "build_info",
        "Build information",
        labels! {
            "git_commit" => match built_info::GIT_VERSION {
                Some(commit) => commit,
                None => "unknown"
            },
        }
    ))
    .unwrap();
}

fn main() -> Result<(), Error> {
    let sys = actix::System::new("policy-engine");

    let settings = config::AppSettings::assemble()?;
    env_logger::Builder::from_default_env()
        .filter(Some(module_path!()), settings.verbosity)
        .filter(Some("cincinnati"), settings.verbosity)
        .init();
    debug!("application settings:\n{:#?}", &settings);

    // Metrics service.
    let registry: &'static Registry = Box::leak(Box::new(metrics::new_registry(Some(
        METRICS_PREFIX.to_string(),
    ))?));
    graph::register_metrics(registry)?;
    registry.register(Box::new(BUILD_INFO.clone()))?;
    HttpServer::new(move || {
        App::new()
            .app_data(actix_web::web::Data::new(RegistryWrapper(registry)))
            .service(
                actix_web::web::resource("/metrics")
                    .route(actix_web::web::get().to(metrics::serve::<RegistryWrapper>)),
            )
    })
    .bind((settings.status_address, settings.status_port))?
    .run();

    // Enable tracing
    let (tracer, span_rx) = Tracer::new(rustracing::sampler::AllSampler);

    // Main service.
    let plugins = settings.validate_and_build_plugins(Some(registry))?;
    let state = AppState {
        mandatory_params: settings.mandatory_client_parameters.clone(),
        path_prefix: settings.path_prefix.clone(),
        plugins: Box::leak(Box::new(plugins)),
        tracer: tracer,
    };

    // Spawns a reporting thread at the initialization phase in your application
    std::thread::spawn(move || {
        let reporter = track_try_unwrap!(JaegerCompactReporter::new("policy-engine"));
        while let Ok(span) = span_rx.recv() {
            if reporter.report(&[span][..]).is_err() {
                break;
            }
        }
    });

    HttpServer::new(move || {
        let app_prefix = state.path_prefix.clone();
        App::new()
            .app_data(actix_web::web::Data::<AppState>::new(state.clone()))
            .service(
                actix_web::web::resource(&format!("{}/v1/graph", app_prefix))
                    .route(actix_web::web::get().to(graph::index)),
            )
            .service(
                actix_web::web::resource(&format!("{}/v1/openapi", app_prefix))
                    .route(actix_web::web::get().to(openapi::index)),
            )
    })
    .bind((settings.address, settings.port))?
    .run();

    BUILD_INFO.inc();

    let _ = sys.run();
    Ok(())
}

/// Shared application configuration (cloned per-thread).
#[derive(Clone, Debug)]
struct AppState {
    /// Query parameters that must be present in all client requests.
    pub mandatory_params: HashSet<String>,
    /// Upstream cincinnati service.
    pub path_prefix: String,
    /// Policy plugins.
    pub plugins: &'static [BoxedPlugin],
    /// Jaeger tracing
    pub tracer: Tracer,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            plugins: Box::leak(Box::new([])),
            mandatory_params: HashSet::new(),
            path_prefix: String::new(),
            tracer: Tracer::new(rustracing::sampler::NullSampler).0,
        }
    }
}
