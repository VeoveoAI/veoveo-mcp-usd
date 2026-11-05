//! `render_stage` as a durable Task. Starting it checks the request, then creates the Task in
//! core's Task runtime. Reading, updating, cancelling and listening go through the SDK's adapter
//! under the caller as the owner.
use std::{collections::BTreeSet, time::Duration};

use rmcp::{
    ErrorData as McpError, RoleServer,
    model::{
        CallToolRequestParams, CreateTaskResult, GetTaskParams, GetTaskResult, UpdateTaskParams,
    },
    service::RequestContext,
};
use veoveo_hub_sdk::{
    HubInternalIdentity,
    calls::ToolCall,
    durable_tasks::{
        DurableTaskService, DurableTaskSubscription, get_durable_task, subscribe_durable_tasks,
        task_seed, update_durable_task,
    },
    hosting::hub_identity,
};
use veoveo_task::{CreateTask, RecoveryClass, TaskError, TaskOwner, TaskRuntime, TaskSurface};
use veoveo_types::TaskId;

use crate::{contract::RenderStageRequest, work::RENDER};

/// How often a client should poll a render's Task.
const POLL_INTERVAL_MS: u64 = 2_000;

/// The server's durable Tasks.
#[derive(Clone)]
pub struct UsdTasks {
    runtime: TaskRuntime,
    retention: Duration,
}

impl UsdTasks {
    /// `retention` is how long a render's Task stays readable.
    pub fn new(runtime: TaskRuntime, retention: Duration) -> Self {
        Self { runtime, retention }
    }
}

fn internal(error: &impl std::fmt::Display) -> McpError {
    tracing::error!(%error, "a usd request failed");
    McpError::internal_error("the server could not complete the request", None)
}

/// The owner of a Task the caller's call starts, and the selection of the caller's own Tasks.
fn task_owner(identity: &HubInternalIdentity) -> Result<TaskOwner, McpError> {
    let actor = &identity.actor;
    Ok(TaskOwner {
        principal_key: actor.id.clone(),
        principal_kind: match actor.kind {
            veoveo_hub_sdk::PrincipalKind::User => veoveo_task::PrincipalKind::User,
            veoveo_hub_sdk::PrincipalKind::Service => veoveo_task::PrincipalKind::Service,
        },
        issuer: actor.issuer.clone(),
        subject: actor.subject.clone(),
        surface: TaskSurface::parse(identity.profile.as_str())
            .map_err(|_| McpError::invalid_request("the Hub profile names no surface", None))?,
        tenant_key: actor.tenant.clone(),
        data_labels: actor.data_labels.clone(),
        authority: identity.authority.clone(),
    })
}

impl DurableTaskService for UsdTasks {
    type Caller = HubInternalIdentity;

    fn authenticate(&self, context: &RequestContext<RoleServer>) -> Result<Self::Caller, McpError> {
        hub_identity(context)
    }

    async fn start_tool_task(
        &self,
        caller: &Self::Caller,
        request: CallToolRequestParams,
    ) -> Result<Option<CreateTaskResult>, McpError> {
        if request.name != RenderStageRequest::TOOL {
            return Ok(None);
        }
        let arguments = serde_json::Value::Object(request.arguments.unwrap_or_default());
        let render: RenderStageRequest = serde_json::from_value(arguments)
            .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        let created = self
            .runtime
            .create(CreateTask {
                task_id: TaskId::new(),
                owner: task_owner(caller)?,
                server: self.runtime.server().clone(),
                task_type: RENDER,
                request: serde_json::to_value(&render).map_err(|error| internal(&error))?,
                recovery_class: RecoveryClass::Resume,
                idempotency_key: None,
                ttl_ms: Some(u64::try_from(self.retention.as_millis()).unwrap_or(u64::MAX)),
                poll_interval_ms: Some(POLL_INTERVAL_MS),
                retention_pins: BTreeSet::new(),
            })
            .await
            .map_err(|error| internal(&error))?;
        Ok(Some(CreateTaskResult::new(task_seed(&created.snapshot))))
    }

    async fn get_task(
        &self,
        caller: &Self::Caller,
        request: GetTaskParams,
    ) -> Result<GetTaskResult, McpError> {
        get_durable_task(
            &self.runtime,
            &self.runtime.for_owner(&task_owner(caller)?),
            request,
        )
        .await
    }

    async fn update_task(
        &self,
        caller: &Self::Caller,
        request: UpdateTaskParams,
    ) -> Result<(), McpError> {
        update_durable_task(&self.runtime.for_owner(&task_owner(caller)?), request).await
    }

    async fn cancel_task(&self, caller: &Self::Caller, task_id: String) -> Result<(), McpError> {
        let unknown = || McpError::invalid_params("unknown task id", None);
        let task = veoveo_task::parse_task_id(&task_id).map_err(|_| unknown())?;
        self.runtime
            .for_owner(&task_owner(caller)?)
            .cancel(task)
            .await
            .map_err(|error| match error {
                TaskError::NotFound(_) | TaskError::WrongServer(_) => unknown(),
                error => internal(&error),
            })?;
        Ok(())
    }

    async fn subscribe_tasks(
        &self,
        caller: &Self::Caller,
        task_ids: Vec<String>,
    ) -> Result<DurableTaskSubscription, McpError> {
        subscribe_durable_tasks(
            &self.runtime,
            &self.runtime.for_owner(&task_owner(caller)?),
            task_ids,
        )
        .await
    }
}
