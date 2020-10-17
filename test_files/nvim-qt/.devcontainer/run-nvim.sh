#!/bin/bash

NVIM_LISTEN_ADDRESS=0.0.0.0:7777

nohup nvim --listen 0.0.0.0:7777 --headless </dev/null &

