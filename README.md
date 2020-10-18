# devcontainers-rs

For those who want to try out VSCode's devcontainers without VSCode.

This project is mostly aiming to bring the VSCode's remote development experience to neovim, but at
the end others editors/clients can be attached too.

Please refer to the devcontainer.json [reference](https://code.visualstudio.com/docs/remote/devcontainerjson-reference)

**⚠️ This is in active development and rapidly growing! Use at your own risk. And feel free to
play around and let me know in the issues what features you'd like to see here. ⚠️**

## Requirements

- docker

## HOW-TO

0- Run `devcontainers_rs -h` to see the available options.

1- Inside a directory containing the `.devcontainer` folder, run:

```bash
$ devcontainers_rs up
```

2- You can add custom settings to be applied to all projects in `$HOME/.config/devcontainer.json`

Available settings: `application` (object), `mounts` (object), `postCreateCommand` (string/array), `postStartCommand` (string/array), `postAttachCommand` (string/array), `forwardPorts` (array), `env` (object)

2.1 - Starting editor/ide after setting up containers:

```json
{
    ...
    "application": { "cmd": ["nvim-qt", "--server", "127.0.0.1:9797", "--nofork"] },
    "forwardPorts": [7777]
    ...
}
```

## FEATURES:

⚙️ - DOING
✅ - DONE

[✅] create containers based on image

[✅] spawn custom application

[✅] `postCreateCommand`, `postStartCommand`, `postAttachCommand`

[✅] `appPort`

[⚙️] `devPort`

[✅] `forwardPorts`

[ ] `initializeCommand`

[✅] create containers based on `build`

[✅] create containers from docker-compose

[✅] stop containers

[ ] destroy containers

[ ] user management (`remoteUser`,  `containerUser`, `updateRemoteUserUID`)

