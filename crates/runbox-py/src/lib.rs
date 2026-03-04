use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use runbox_core::{BindingResolver, GitContext, Storage};
use std::collections::HashMap;
use std::process::Command;

/// Convert anyhow::Error to PyErr
fn to_py_err(e: anyhow::Error) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

/// Run a template with the given bindings
#[pyfunction]
#[pyo3(signature = (template_id, bindings=None))]
fn run_template(template_id: &str, bindings: Option<HashMap<String, String>>) -> PyResult<String> {
    let storage = Storage::new().map_err(to_py_err)?;
    let template = storage.load_template(template_id).map_err(to_py_err)?;

    // Build binding list
    let binding_list: Vec<String> = bindings
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();

    let resolver = BindingResolver::new().with_bindings(binding_list);

    // Get git context
    let git = GitContext::from_current_dir().map_err(to_py_err)?;
    let temp_run_id = format!("run_{}", uuid::Uuid::new_v4());
    let code_state = git.build_code_state(&temp_run_id).map_err(to_py_err)?;

    // Build and save run
    let run = resolver
        .build_run(&template, code_state)
        .map_err(to_py_err)?;
    run.validate()
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

    storage.save_run(&run).map_err(to_py_err)?;

    // Execute
    let status = Command::new(&run.exec.argv[0])
        .args(&run.exec.argv[1..])
        .current_dir(&run.exec.cwd)
        .envs(&run.exec.env)
        .status()
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

    if !status.success() {
        return Err(PyRuntimeError::new_err(format!(
            "Command failed with status: {:?}",
            status.code()
        )));
    }

    Ok(run.run_id)
}

/// List all templates
#[pyfunction]
fn list_templates() -> PyResult<Vec<HashMap<String, String>>> {
    let storage = Storage::new().map_err(to_py_err)?;
    let templates = storage.list_templates().map_err(to_py_err)?;

    Ok(templates
        .into_iter()
        .map(|t| {
            let mut m = HashMap::new();
            m.insert("template_id".to_string(), t.template_id);
            m.insert("name".to_string(), t.name);
            m
        })
        .collect())
}

/// Get a template by ID
#[pyfunction]
fn get_template(template_id: &str) -> PyResult<String> {
    let storage = Storage::new().map_err(to_py_err)?;
    let template = storage.load_template(template_id).map_err(to_py_err)?;
    serde_json::to_string_pretty(&template).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

/// List all runs
#[pyfunction]
#[pyo3(signature = (limit=None))]
fn list_runs(limit: Option<usize>) -> PyResult<Vec<HashMap<String, String>>> {
    let storage = Storage::new().map_err(to_py_err)?;
    let runs = storage.list_runs(limit.unwrap_or(10)).map_err(to_py_err)?;

    Ok(runs
        .into_iter()
        .map(|r| {
            let mut m = HashMap::new();
            m.insert("run_id".to_string(), r.run_id);
            m.insert("command".to_string(), r.exec.argv.join(" "));
            m.insert("commit".to_string(), r.code_state.base_commit);
            m
        })
        .collect())
}

/// Get a run by ID
#[pyfunction]
fn get_run(run_id: &str) -> PyResult<String> {
    let storage = Storage::new().map_err(to_py_err)?;
    let run = storage.load_run(run_id).map_err(to_py_err)?;
    serde_json::to_string_pretty(&run).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

/// Replay a previous run
#[pyfunction]
fn replay(run_id: &str) -> PyResult<()> {
    let storage = Storage::new().map_err(to_py_err)?;
    let run = storage.load_run(run_id).map_err(to_py_err)?;

    let status = Command::new(&run.exec.argv[0])
        .args(&run.exec.argv[1..])
        .current_dir(&run.exec.cwd)
        .envs(&run.exec.env)
        .status()
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;

    if !status.success() {
        return Err(PyRuntimeError::new_err(format!(
            "Replay failed with status: {:?}",
            status.code()
        )));
    }

    Ok(())
}

/// List all playlists
#[pyfunction]
fn list_playlists() -> PyResult<Vec<HashMap<String, String>>> {
    let storage = Storage::new().map_err(to_py_err)?;
    let playlists = storage.list_playlists().map_err(to_py_err)?;

    Ok(playlists
        .into_iter()
        .map(|p| {
            let mut m = HashMap::new();
            m.insert("playlist_id".to_string(), p.playlist_id);
            m.insert("name".to_string(), p.name);
            m.insert("item_count".to_string(), p.items.len().to_string());
            m
        })
        .collect())
}

/// Get a playlist by ID
#[pyfunction]
fn get_playlist(playlist_id: &str) -> PyResult<String> {
    let storage = Storage::new().map_err(to_py_err)?;
    let playlist = storage.load_playlist(playlist_id).map_err(to_py_err)?;
    serde_json::to_string_pretty(&playlist).map_err(|e| PyRuntimeError::new_err(e.to_string()))
}

/// Validate a JSON string
#[pyfunction]
fn validate(json_str: &str) -> PyResult<String> {
    let validator = runbox_core::Validator::new().map_err(to_py_err)?;
    let value: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    let validation_type = validator.validate_auto(&value).map_err(to_py_err)?;
    Ok(validation_type.to_string())
}

/// Python module
#[pymodule]
fn runbox(_py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(run_template, m)?)?;
    m.add_function(wrap_pyfunction!(list_templates, m)?)?;
    m.add_function(wrap_pyfunction!(get_template, m)?)?;
    m.add_function(wrap_pyfunction!(list_runs, m)?)?;
    m.add_function(wrap_pyfunction!(get_run, m)?)?;
    m.add_function(wrap_pyfunction!(replay, m)?)?;
    m.add_function(wrap_pyfunction!(list_playlists, m)?)?;
    m.add_function(wrap_pyfunction!(get_playlist, m)?)?;
    m.add_function(wrap_pyfunction!(validate, m)?)?;
    Ok(())
}
