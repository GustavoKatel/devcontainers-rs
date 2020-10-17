#!/bin/bash

set -e
set -x

apt-get update

apt-get upgrade -y

apt-get install -y neovim python3-neovim
