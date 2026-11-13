#!/bin/sh
[ -n "$INCLUDE_ONLY" ] || {
	. ../netifd-proto.sh
	init_proto "$@"
}

proto_wlmb_air6_init_config() {
	no_device=1
	available=1
	teardown_on_l3_link_down=1

	proto_config_add_string apn
	proto_config_add_string username
	proto_config_add_string password
	proto_config_add_string iaapn
	proto_config_add_int auth
	proto_config_add_int mtu
	proto_config_add_int iptype
	proto_config_add_int roamingtype
	proto_config_add_int rattype
	proto_config_add_int dataroaming
	proto_config_add_int auto_conf
	proto_config_add_defaults
}

proto_wlmb_air6_setup() {
	exec /usr/sbin/wlmb-server --air6-netifd setup "$1" "$data"
}

proto_wlmb_air6_teardown() {
	exec /usr/sbin/wlmb-server --air6-netifd teardown "$1" '{}'
}

[ -n "$INCLUDE_ONLY" ] || add_protocol wlmb_air6
