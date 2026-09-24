# dmgbuild settings for the Osprey installer window. Paths arrive through -D defines from
# scripts/make-dmg.sh. Icon centres match scripts/render-dmg-background.swift.
import os.path

app = defines["app"]
app_name = os.path.basename(app)

format = "UDZO"
compression_level = 9
filesystem = "APFS"
files = [app]
symlinks = {"Applications": "/Applications"}
icon = defines.get("icon")
background = defines.get("background")

window_rect = ((200, 140), (660, 440))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
show_icon_preview = False
icon_size = 112
text_size = 13
arrange_by = None
icon_locations = {app_name: (170, 232), "Applications": (490, 232)}
