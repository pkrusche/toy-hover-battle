.PHONY: all render sfx explosion rocks ship rocket install update-assets clean

all: render sfx

render: explosion rocks ship rocket

sfx:
	./sfx/generate.sh

explosion:
	./explosion/render.sh

rocks:
	./rocks/render.sh

ship:
	./ship/render.sh

rocket:
	./rocket/render.sh

# Copy the current render and sound output into the game's assets directory.
# Override the destination with GAME_ASSETS_DIR; pass INSTALL_ARGS to select
# individual assets or to allow a sprite-sheet size change, for example
# `make install INSTALL_ARGS="ship --force"`.
install:
	./scripts/install-assets.sh $(INSTALL_ARGS)

# Regenerate everything, then update the game's assets.
update-assets: all install

clean:
	rm -rf frames
