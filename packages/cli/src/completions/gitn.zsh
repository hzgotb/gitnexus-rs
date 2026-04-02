#compdef gitn

_gitn() {
  local -a commands
  local help_desc command_label args_label shell_label
  local doctor_fix_desc doctor_registry_desc doctor_repos_desc
  local start_tag_desc start_port_desc start_registry_desc start_repos_desc start_rebuild_desc
  local analyze_container_desc analyze_registry_desc analyze_repo_desc analyze_all_desc analyze_jobs_desc analyze_passthrough_desc
  local add_restart_desc add_rebuild_desc add_analyze_desc add_container_desc add_repos_desc add_registry_desc source_dir_label dest_name_label
  local completion_target_label

  commands=(
    'doctor:Environment diagnostics (docker/colima + json files)'
    'diagnose:Alias of doctor'
    'start:Start or rebuild a container'
    'analyze:Run gitnexus analyze inside a container'
    'add-repo:Add a repo mapping'
    'add_repo:Alias of add-repo'
    'completion:Print shell completion script'
    'complete:Alias of completion'
    'help:Show help'
  )
  help_desc='Show help'
  command_label='command'
  args_label='args'
  shell_label='shell'
  doctor_fix_desc='Create missing registry.json / repos.json'
  doctor_registry_desc='Override registry.json path'
  doctor_repos_desc='Override repos.json path'
  start_tag_desc='Set image tag'
  start_port_desc='Bind host port'
  start_registry_desc='Override registry.json path'
  start_repos_desc='Override repos.json path'
  start_rebuild_desc='Rebuild the specified container directly'
  analyze_container_desc='Specify target container'
  analyze_registry_desc='Override registry.json path'
  analyze_repo_desc='Specify target repo under /repos'
  analyze_all_desc='Run analyze for all repos'
  analyze_jobs_desc='Parallel jobs for --all'
  analyze_passthrough_desc='Pass remaining args to gitnexus analyze'
  add_restart_desc='Rebuild after writing repos.json'
  add_rebuild_desc='Alias of --restart'
  add_analyze_desc='Rebuild and analyze after writing'
  add_container_desc='Specify target container'
  add_repos_desc='repos.json to update'
  add_registry_desc='registry.json for follow-up start'
  source_dir_label='source dir'
  dest_name_label='dest name'
  completion_target_label='shell'

  local cmd=""
  local cmd_index=0
  local i=2
  while (( i <= $#words )); do
    case "${words[i]}" in
      -h|--help)
        (( i += 1 ))
        continue
        ;;
      *)
        cmd="${words[i]}"
        cmd_index=$i
        break
        ;;
    esac
  done

  if [[ -z "$cmd" || CURRENT <= cmd_index || $cmd_index -eq 0 ]]; then
    _arguments -s \
      "(-h --help)"{-h,--help}"[$help_desc]" \
      "1:$command_label:->command" \
      "*::$args_label:->args"
    if [[ "$state" == "command" ]]; then
      _describe -t commands 'gitn commands' commands
    fi
    return
  fi

  case "$cmd" in
    doctor|diagnose)
      _arguments -s \
        "(-h --help)"{-h,--help}"[$help_desc]" \
        "(-r --registry)"{-r,--registry}"[$doctor_registry_desc]:registry:_files" \
        "--repos[$doctor_repos_desc]:repos:_files" \
        "--fix[$doctor_fix_desc]"
      ;;
    start)
      _arguments -s \
        "(-h --help)"{-h,--help}"[$help_desc]" \
        "(-t --tag)"{-t,--tag}"[$start_tag_desc]:tag:" \
        "(-p --port)"{-p,--port}"[$start_port_desc]:port:" \
        "(-r --registry)"{-r,--registry}"[$start_registry_desc]:registry:_files" \
        "--repos[$start_repos_desc]:repos:_files" \
        "--rebuild[$start_rebuild_desc]:container:"
      ;;
    analyze)
      _arguments -s \
        "(-h --help)"{-h,--help}"[$help_desc]" \
        "(-c --container)"{-c,--container}"[$analyze_container_desc]:container:" \
        "--registry[$analyze_registry_desc]:registry:_files" \
        "(-r --repo)"{-r,--repo}"[$analyze_repo_desc]:repo:" \
        "(-a --all)"{-a,--all}"[$analyze_all_desc]" \
        "(-j --jobs)"{-j,--jobs}"[$analyze_jobs_desc]:jobs:" \
        "--[$analyze_passthrough_desc]"
      ;;
    add-repo|add_repo)
      _arguments -s \
        "(-h --help)"{-h,--help}"[$help_desc]" \
        "--restart[$add_restart_desc]" \
        "--rebuild[$add_rebuild_desc]" \
        "--analyze[$add_analyze_desc]" \
        "(-c --container)"{-c,--container}"[$add_container_desc]:container:" \
        "--repos[$add_repos_desc]:repos:_files" \
        "--registry[$add_registry_desc]:registry:_files" \
        "1:$source_dir_label:_files -/" \
        "2:$dest_name_label:"
      ;;
    completion|complete)
      _arguments -s \
        "(-h --help)"{-h,--help}"[$help_desc]" \
        "1:$completion_target_label:(zsh)"
      ;;
    help)
      return 0
      ;;
    *)
      _default
      ;;
  esac
}

compdef _gitn gitn
