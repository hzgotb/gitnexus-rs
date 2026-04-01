#compdef gitn

_gitn_detect_lang() {
  local value lowered
  for value in "$GITN_LANG" "$LC_ALL" "$LC_MESSAGES" "$LANG"; do
    [[ -z "$value" ]] && continue
    lowered="${(L)value}"
    if [[ "$value" == *中文* || "$lowered" == *zh* ]]; then
      print -r -- zh
      return
    fi
    if [[ "$lowered" == *en* ]]; then
      print -r -- en
      return
    fi
  done
  print -r -- en
}

_gitn() {
  local lang="$(_gitn_detect_lang)"
  local -a commands
  local word_lang help_desc command_label args_label shell_label language_label
  local doctor_fix_desc doctor_registry_desc doctor_repos_desc
  local start_tag_desc start_port_desc start_registry_desc start_repos_desc start_rebuild_desc
  local analyze_container_desc analyze_registry_desc analyze_repo_desc analyze_all_desc analyze_jobs_desc analyze_passthrough_desc
  local add_restart_desc add_rebuild_desc add_analyze_desc add_container_desc add_repos_desc add_registry_desc source_dir_label dest_name_label
  local completion_target_label

  if [[ "$lang" == "zh" ]]; then
    commands=(
      'doctor:环境诊断（检查 docker/colima 与 json 文件）'
      'diagnose:doctor 的别名'
      '诊断:doctor 的中文别名'
      'start:启动或重建容器'
      'analyze:在容器内执行 gitnexus analyze'
      'add-repo:添加仓库映射'
      'add_repo:add-repo 的别名'
      'completion:输出 shell 补全脚本'
      'complete:completion 的别名'
      'help:显示帮助'
      '帮助:help 的中文别名'
    )
    word_lang='语言'
    help_desc='显示帮助'
    command_label='命令'
    args_label='参数'
    shell_label='shell'
    language_label='设置界面语言'
    doctor_fix_desc='自动创建缺失的 registry.json / repos.json'
    doctor_registry_desc='指定 registry.json 路径'
    doctor_repos_desc='指定 repos.json 路径'
    start_tag_desc='指定镜像 tag'
    start_port_desc='绑定宿主机端口'
    start_registry_desc='指定 registry.json 路径'
    start_repos_desc='指定 repos.json 路径'
    start_rebuild_desc='直接重建指定旧容器'
    analyze_container_desc='指定目标容器'
    analyze_registry_desc='指定 registry.json 路径'
    analyze_repo_desc='指定 /repos 下目标仓库'
    analyze_all_desc='对所有仓库执行 analyze'
    analyze_jobs_desc='--all 并发数'
    analyze_passthrough_desc='后续参数透传给 gitnexus analyze'
    add_restart_desc='写入后重建容器'
    add_rebuild_desc='--restart 的别名'
    add_analyze_desc='写入后重建容器并执行 analyze'
    add_container_desc='指定目标容器'
    add_repos_desc='指定要写入的 repos.json'
    add_registry_desc='指定后续 start 使用的 registry.json'
    source_dir_label='源目录'
    dest_name_label='目标名称'
    completion_target_label='shell'
  else
    commands=(
      'doctor:Environment diagnostics (docker/colima + json files)'
      'diagnose:Alias of doctor'
      '诊断:Chinese alias of doctor'
      'start:Start or rebuild a container'
      'analyze:Run gitnexus analyze inside a container'
      'add-repo:Add a repo mapping'
      'add_repo:Alias of add-repo'
      'completion:Print shell completion script'
      'complete:Alias of completion'
      'help:Show help'
      '帮助:Chinese alias of help'
    )
    word_lang='language'
    help_desc='Show help'
    command_label='command'
    args_label='args'
    shell_label='shell'
    language_label='Set UI language'
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
  fi

  if [[ "${words[CURRENT-1]}" == "--lang" ]]; then
    _values "$word_lang" zh en
    return
  fi

  local cmd=""
  local cmd_index=0
  local i=2
  while (( i <= $#words )); do
    case "${words[i]}" in
      --lang)
        (( i += 2 ))
        continue
        ;;
      --lang=*)
        (( i += 1 ))
        continue
        ;;
      --zh|--en)
        (( i += 1 ))
        continue
        ;;
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
      "--lang[$language_label]:$word_lang:(zh en)" \
      '--zh[Set UI language to Chinese]' \
      '--en[Set UI language to English]' \
      "(-h --help)"{-h,--help}"[$help_desc]" \
      "1:$command_label:->command" \
      "*::$args_label:->args"
    if [[ "$state" == "command" ]]; then
      _describe -t commands 'gitn commands' commands
    fi
    return
  fi

  case "$cmd" in
    doctor|diagnose|诊断)
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
    help|帮助)
      return 0
      ;;
    *)
      _default
      ;;
  esac
}

compdef _gitn gitn
