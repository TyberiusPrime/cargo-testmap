# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_cargo_testmap_global_optspecs
    string join \n h/help V/version
end

function __fish_cargo_testmap_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_cargo_testmap_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_cargo_testmap_using_subcommand
    set -l cmd (__fish_cargo_testmap_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c cargo-testmap -n "__fish_cargo_testmap_needs_command" -s h -l help -d 'Print help'
complete -c cargo-testmap -n "__fish_cargo_testmap_needs_command" -s V -l version -d 'Print version'
complete -c cargo-testmap -n "__fish_cargo_testmap_needs_command" -f -a "collect" -d 'Collect per-test coverage and build a testmap.json database.'
complete -c cargo-testmap -n "__fish_cargo_testmap_needs_command" -f -a "report" -d 'Generate an HTML report from a testmap.json database.'
complete -c cargo-testmap -n "__fish_cargo_testmap_needs_command" -f -a "run" -d 'Collect coverage and build the report in one go (collect then report).'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -s p -l package -d 'Specific package to collect.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -l filter -d 'Only collect tests whose full path matches this regex.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -l skip -d 'Skip tests whose full path matches this regex.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -l threshold -d 'Omit lines covered by >= N tests.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -s j -l jobs -d 'Number of parallel test runs (default: number of CPUs).' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -l output -d 'Database output path.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -l workspace -d 'Collect across all workspace members (default).'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -l lib -d 'Include library targets.'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -l bins -d 'Include binary targets.'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -l tests -d 'Include test targets.'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -s v -l verbose -d 'Show additional output.'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand collect" -s h -l help -d 'Print help'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand report" -l input -d 'Database input path.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand report" -l output-dir -d 'Report output directory.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand report" -l theme -d 'Syntax-highlighting theme.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand report" -l single-file -d 'Generate a single self-contained HTML file.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand report" -s h -l help -d 'Print help'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -s p -l package -d 'Specific package to collect.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -l filter -d 'Only collect tests whose full path matches this regex.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -l skip -d 'Skip tests whose full path matches this regex.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -l threshold -d 'Omit lines covered by >= N tests.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -s j -l jobs -d 'Number of parallel test runs (default: number of CPUs).' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -l output -d 'Database path: collect writes here, report reads here.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -l output-dir -d 'Report output directory.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -l theme -d 'Syntax-highlighting theme.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -l single-file -d 'Generate a single self-contained HTML file.' -r
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -l workspace -d 'Collect across all workspace members (default).'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -l lib -d 'Include library targets.'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -l bins -d 'Include binary targets.'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -l tests -d 'Include test targets.'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -s v -l verbose -d 'Show additional output.'
complete -c cargo-testmap -n "__fish_cargo_testmap_using_subcommand run" -s h -l help -d 'Print help'
