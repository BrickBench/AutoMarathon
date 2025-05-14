import { formUrl } from "../Api";
import { Event, StreamEntry, StreamHost, StreamRunnersEntry } from "../websocket";

export function getSelectedLayout(host: StreamHost) {
  let s = Object.entries(host.scenes).find(([_, scene]) => scene.active);
  return s ? s[0] : "";
}

export function getStreamForHost(streams: StreamEntry[], selected_host: string) {
  let stream = streams.find(element => element.obs_host == selected_host);
  return structuredClone(stream);
}

export function autoFillLayout(event: Event, host: StreamHost): [string | undefined, StreamRunnersEntry] {
  var foundScene = undefined;
  for (let layout of event.preferred_layouts) {
    if (layout in host.scenes) {
      foundScene = host.scenes[layout];
      break;
    }
  }
  if (!foundScene) {
    return [foundScene, {}];
  }
  var sources = Object.entries(foundScene.sources).map(([key, _value]) => { return key.toString() }).sort();
  var players = Object.entries(event.runner_state).map(([key, _value]) => { return key.toString() }).sort();

  var streamMap: StreamRunnersEntry = {};

  var index = 0;
  for (let source of sources) {
    if (index >= players.length) {
      break;
    }
    streamMap[source] = parseInt(players[index]);
    index++;
  }
  return [foundScene.name, streamMap];
}

function getBestLayout(host: StreamHost, numPlayers: number, event: Event) {
  var playerNumToScene = new Map<number, string[]>();
  for (let entry of Object.entries(host.scenes)) {
    let numSources = Object.keys(entry[1].sources).length;
    playerNumToScene.set(numSources, playerNumToScene.get(numSources) || []);
    playerNumToScene.get(numSources)!.push(entry[0]);
  }

  let compatibleScenes = playerNumToScene.get(numPlayers);

  if (compatibleScenes) {
    let selectedLayout = compatibleScenes[0];
    let bestScore = 100000000000;

    if (event.preferred_layouts) {
      for (let layout of compatibleScenes) {
        let index = event.preferred_layouts.findIndex((lay) => layout == lay);
        if (index != -1 && index < bestScore) {
          selectedLayout = layout;
          bestScore = index;
        }
      }
    }

    return selectedLayout;
  } else {
    return undefined;
  }
}

export function addPlayerSelectLayout(host: StreamHost, stream: StreamEntry, currentLayout: string, player: number, event: Event): [StreamEntry, string, string | undefined, string | undefined] {
  var newstate = { ...stream };
  var errormessage;
  var warning;

  let maxRunnersCurrLayout = Object.keys(host.scenes[currentLayout].sources).length;
  let bestLayout: string = currentLayout;

  //If we have empty slots on the current layout, fill into those first.
  let firstEmptySlot = undefined;
  for (let index = 1; index <= maxRunnersCurrLayout; index++) {
    if (!stream.stream_runners[index]) {
      firstEmptySlot = index;
      break;
    }
  }

  if (firstEmptySlot) {
    newstate.stream_runners[firstEmptySlot] = player;
  } else {
    let bestNextLayout = getBestLayout(host, maxRunnersCurrLayout + 1, event);
    if (bestNextLayout) {
      bestLayout = bestNextLayout;
      let existingRunners = Object.entries(stream.stream_runners).sort().map(([_key, value]) => value);
      let newRunners = [];
      for (let index = 0; index <= Math.max(existingRunners.length - 1, maxRunnersCurrLayout); index++) {
        if (index < maxRunnersCurrLayout && index < existingRunners.length) {
          newRunners.push(existingRunners[index]);
        }
        if (index == maxRunnersCurrLayout) {
          newRunners.push(player);
        }
        if (index >= maxRunnersCurrLayout && index < existingRunners.length) {
          if (existingRunners[index] != player) {
            newRunners.push(existingRunners[index]);
          }
        }
      }
      newstate.stream_runners = Object.fromEntries(newRunners.map((runner, index) => [(index + 1).toString(), runner]));
    } else {
      errormessage = `No valid layouts with ${maxRunnersCurrLayout + 1} players.`;
    }
  }

  if (maxRunnersCurrLayout == 0) {
    newstate.audible_runner = player;
  }

  return [newstate, bestLayout, warning, errormessage];
}

export function removePlayerSelectLayout(host: StreamHost, stream: StreamEntry, currentLayout: string, removeSlot: string, event: Event): [StreamEntry, string | undefined, string | undefined, string | undefined] {
  var newstate = { ...stream };
  var errormessage;
  var warning;

  let numOnScreen = Object.keys(host.scenes[currentLayout].sources).length;
  let bestLayout = getBestLayout(host, numOnScreen - 1, event);

  let runnerToRemove = newstate.stream_runners[removeSlot];

  let removeIndex = parseInt(removeSlot);

  if (bestLayout) {
    if (stream.stream_runners[removeSlot]) {
      let streamRunnersEntries = Object.entries(stream.stream_runners).sort();
      let newStreamRunnersEntries: [number, number][] = []
      for (let entry of streamRunnersEntries) {
        let entryNum = parseInt(entry[0]);
        if (entryNum < removeIndex) {
          newStreamRunnersEntries.push([entryNum, entry[1]]);
        } else if (entryNum > removeIndex) {
          newStreamRunnersEntries.push([entryNum - 1, entry[1]]);
        }
      }
      newstate.stream_runners = Object.fromEntries(newStreamRunnersEntries.map<[string, number]>(([key, val]) => [key.toString(), val]));

      if (runnerToRemove == newstate.audible_runner && !newStreamRunnersEntries.find(([key, val]) => ((val == runnerToRemove) && (key < numOnScreen - 1)))) {
        newstate.audible_runner = newStreamRunnersEntries[0][1];
      }
    } else {
      let streamRunnersEntries = Object.entries(stream.stream_runners).sort();
      let newStreamRunnersEntries: [number, number][] = []
      for (let entry of streamRunnersEntries) {
        let entryNum = parseInt(entry[0]);
        if (entryNum < removeIndex) {
          newStreamRunnersEntries.push([entryNum, entry[1]]);
        } else if (entryNum > removeIndex) {
          newStreamRunnersEntries.push([entryNum - 1, entry[1]]);
        }
      }
      newstate.stream_runners = Object.fromEntries(newStreamRunnersEntries.map<[string, number]>(([key, val]) => [key.toString(), val]));
    }
  } else {
    errormessage = `No valid layouts with ${numOnScreen - 1} players.`;
  }

  return [newstate, bestLayout, warning, errormessage];
}

export function swapPlayerPlayerLayout(_host: StreamHost, stream: StreamEntry, position1: string, position2: string): [StreamEntry, string | undefined, string | undefined] {
  var newstate = { ...stream };
  var errormessage;
  var warning;
  if (!newstate.stream_runners[position1] && !newstate.stream_runners[position2]) {
  } else if (!newstate.stream_runners[position1]) {
    newstate.stream_runners[position1] = newstate.stream_runners[position2];
    delete newstate.stream_runners[position2];
  } else if (!newstate.stream_runners[position2]) {
    newstate.stream_runners[position2] = newstate.stream_runners[position1];
    delete newstate.stream_runners[position1];
  } else {
    let temp = newstate.stream_runners[position1];
    newstate.stream_runners[position1] = newstate.stream_runners[position2];
    newstate.stream_runners[position2] = temp;
  }
  return [newstate, warning, errormessage];
}

export function updateStreamRequest(oldeventid: number, newstream: StreamEntry, callback: (s: boolean) => void) {
  console.log("request", oldeventid, newstream);
  if (oldeventid) {
    if (oldeventid == newstream.event) {
      console.log("Doing PUT request");
      (async () => {
        await fetch(formUrl('/stream'), {
          method: 'PUT',
          headers: {
            'Access-Control-Allow-Origin': '*',
            'Content-Type': 'application/json'
          },
          body: JSON.stringify(newstream)
        }).then((response) => {
          if (response.status >= 400 && response.status < 600) {
            throw new Error("Bad response from server");
          }
          return response;
        }).then((_returnedResponse) => {
          // Your response to manipulate
          if (callback) callback(true);
        }).catch((error) => {
          // Your error is here!
          if (callback) callback(false);
          console.log(error)
        });
      })();
    } else {
      console.log("Doing DELETE POST request");
      (async () => {
        await fetch(formUrl('/stream'), {
          method: 'DELETE',
          headers: {
            'Access-Control-Allow-Origin': '*',
            'Content-Type': 'application/json'
          },
          body: JSON.stringify({
            id: oldeventid
          })
        }).then((response) => {
          if (response.status >= 400 && response.status < 600) {
            throw new Error("Bad response from server");
          }
          return response;
        }).then((_returnedResponse) => {
          // Your response to manipulate
          (async () => {
            await fetch(formUrl('/stream'), {
              method: 'POST',
              headers: {
                'Access-Control-Allow-Origin': '*',
                'Content-Type': 'application/json'
              },
              body: JSON.stringify(newstream)
            }).then((response) => {
              if (response.status >= 400 && response.status < 600) {
                throw new Error("Bad response from server 2");
              }
              return response;
            }).then((_returnedResponse) => {
              // Your response to manipulate
              if (callback) callback(true);

            }).catch((error) => {
              // Your error is here!
              console.log(error)
              if (callback) callback(false);
            });
          })();
        }).catch((error) => {
          // Your error is here!
          console.log(error)
          if (callback) callback(false);
        });

      })();
    }
  } else {
    console.log("Doing POST request");
    (async () => {
      await fetch(formUrl('/stream'), {
        method: 'POST',
        headers: {
          'Access-Control-Allow-Origin': '*',
          'Content-Type': 'application/json'
        },
        body: JSON.stringify(newstream)
      }).then((response) => {
        if (response.status >= 400 && response.status < 600) {
          throw new Error("Bad response from server");
        }
        return response;
      }).then((_returnedResponse) => {
        // Your response to manipulate
        if (callback) callback(true);
      }).catch((error) => {
        // Your error is here!
        if (callback) callback(false);
        console.log(error)
      });
    })();
  }

}
