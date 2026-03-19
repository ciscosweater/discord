id := "me.amankhanna.oadiscord.sdPlugin"
linux_bin := "target/release/oadiscord"
linux_bin_name := "oadiscord-x86_64-unknown-linux-gnu"

default: build

build:
    cargo build --release

build-pi:
    npm run build --prefix pi

package: build build-pi
    rm -rf build && mkdir -p build/{{id}} && cp -r assets/* build/{{id}}/ && cp {{linux_bin}} build/{{id}}/{{linux_bin_name}} && cd build && zip -r {{id}}.zip {{id}}/

clean:
    sudo rm -rf target/

# Install to OpenDeck plugins directory
install: package
    unzip -o build/{{id}}.zip -d ~/.config/opendeck/plugins/
