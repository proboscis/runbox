package runbox

// Run: A fully-resolved, reproducible execution record
#Run: {
    run_version: 0
    run_id:      =~"^run_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
    exec:        #Exec
    code_state:  #CodeState
}

#Exec: {
    argv:        [...string] & [_, ...]  // non-empty array
    cwd:         string                   // relative to repo root
    env:         [string]: string
    timeout_sec: int & >=0 | *0          // 0 = unlimited
}

#CodeState: {
    repo_url:    string
    base_commit: =~"^[a-f0-9]{40}$"
    patch?:      #Patch
}

#Patch: {
    ref:    =~"^refs/patches/"
    sha256: =~"^[a-f0-9]{64}$"
}

// RunTemplate: A template for creating Runs with variable bindings
#RunTemplate: {
    template_version: 0
    template_id:      =~"^tpl_"
    name:             string

    exec: {
        argv:        [...string]           // template variables allowed: "{i}", "{seed}"
        cwd:         string
        env:         [string]: string
        timeout_sec: int & >=0 | *0
    }

    bindings?: {
        defaults?:    [string]: _          // default values
        interactive?: [...string]          // prompt user at runtime
    }

    code_state: {
        repo_url: string
        // base_commit: resolved at runtime from HEAD
        // patch: captured at runtime from git diff
    }
}

// Playlist: A collection of RunTemplate references
#Playlist: {
    playlist_id: =~"^pl_"
    name:        string
    items:       [...#PlaylistItem]
}

#PlaylistItem: {
    template_id: string
    label?:      string  // display name override
}
