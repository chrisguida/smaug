enter_service() {
    name=$1
    pid=$(systemctl show -p MainPID --value "$name")
    IFS=- read -r uid gid  < <(stat -c "%u-%g" "/proc/$pid")
    nsenter --all -t "$pid" --setuid "$uid" --setgid "$gid" bash
}

