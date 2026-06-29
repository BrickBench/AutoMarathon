import { StreamEntry, StreamHost, Event, Person, Runner, CustomFields, StreamRunnersEntry, RunnerStateEntry } from "../websocket";
import { WebUIStateContext } from "../Context";
import { Button, Col, ListGroup, ListGroupItem, Row } from "react-bootstrap";
import { ArrowClockwise, DashLg } from "react-bootstrap-icons";
import { customStyles } from "../Globals";
import Select from "react-select";
import { useState } from "react";
import { doPost } from "../Api";

function PlayerStreamItem({streamRunnerEntry, stream, runners, people, event, setStreamState} 
    : {streamRunnerEntry: [string, number], stream : StreamEntry, runners: Map<number, Runner>, people: Map<number, Person>,
        event: Event, setStreamState : React.Dispatch<React.SetStateAction<StreamEntry>>}){
    let selectOptions = stream ? Array.from(Object.entries(event.runner_state)
        .map<[string, RunnerStateEntry, boolean]>(([key, runnerEntry]) => {
          let index = Object.entries(stream.stream_runners).findIndex(([_, val2]) => { return parseInt(key) == val2 });
          return [key, runnerEntry, (index == -1 || index >= numOnScreen)];
        }).sort(([key, runner1, bad], [key2, runner2, bad2]) => {
          if (bad && bad2) {
            return people.get(runner1.runner)!.name.localeCompare(people.get(runner2.runner)!.name);
          } else if (bad) {
            return -1;
          } else if (bad2) {
            return 1;
          } else {
            return people.get(runner1.runner)!.name.localeCompare(people.get(runner2.runner)!.name);
          }
        })) : [];
    
    let selectOptionsPlayerChecks = Array.from(selectOptions, ([key, runnerEntry, good]) => ({ value: runnerEntry.runner, label: (!good ? "✓" : "") + people.get(runnerEntry.runner)!.name }))
    let runner = runners.get(streamRunnerEntry[1]);
    let selectedRunner = stream ? streamRunnerEntry[1] : undefined;
    let selectedRunnerNum = selectedRunner ? selectedRunner : -1;
    let selectedRunnerName = selectedRunner ? people.get(selectedRunnerNum)!.name : '';
    
    return <li className="list-group-item d-flex justify-content-between align-items-start">
                <div className="ms-2 me-auto">
                <div className="fw-bold">
                    <Select styles={customStyles} value={runner ? { value: runner, label: people.get(selectedRunnerNum)!.name } : undefined}
                    options={selectOptionsPlayerChecks}
                    onChange={selectedOption => {
                      let layout_temp = { ...stream.stream_runners };

                      let sourceStr = streamRunnerEntry[0].toString();

                      delete layout_temp[sourceStr];

                      if (selectedOption) {
                        layout_temp[sourceStr] = selectedOption.value;
                      }

                      setStreamState({
                        ...stream,
                        stream_runners: layout_temp,
                      });
                    }}
                  ></Select></div>
                <div>
                    {runner.stream_urls['best'] ?? "Blank"}
                    <Button variant={((runner.stream_urls && (Object.keys(runner.stream_urls).length != 0)) ? "primary" : "warning")} type="button" id="button-refreshlink" onClick={() => {
                        doPost('runner/refresh', 'POST', {
                            id: runner.participant
                        });
                    }}>Refresh Streamlink<ArrowClockwise /></Button>
                </div>
                </div>
                <Button type="button" variant="danger" className="p-0 align-self-end" style={{ width: "1.5rem", height: "1.5rem" }}
                        onClick={() => {
                            let [newstate, bestlayout, warning, errormessage] = removePlayerSelectLayout(host, streamState, selectedLayout, sourceIndex, selectedEvent);
                            if (errormessage) {
                            alert(errormessage);
                            } else if (warning) {
                            var result = confirm(warning + " Do you want to continue?");
                            if (result) {
                              setStreamState(newstate);
                            }
                            } else {
                              setStreamState(newstate);
                            }
                        }}>
                        <DashLg/>
                </Button>
            </li>;
}
export function HeadlessEventPlayerList({ event, people, stream, runners }: {
  event: Event, people: Map<number, Person>, stream: StreamEntry,
  runners: Map<number, Runner>}
){
    let [streamState, setStreamState] = useState<StreamEntry>(stream);
    let entries = Object.entries(streamState.stream_runners);

    return <Col>
        <Row>
            <Button variant="submit">Add Entry</Button>
        </Row>
        <Row>
            <ListGroup numbered={true}>
                {entries.map((e) => {
                    return <PlayerStreamItem streamRunnerEntry={e}
                        stream={streamState} runners={runners}
                        people={people} event={event} setStreamState={setStreamState}
                    ></PlayerStreamItem>;
                })}
            </ListGroup>
        </Row>
        <Row>
            <Button variant="submit" onClick={()=>{
                doPost('stream', 'PUT', streamState);
            }}>Save Entries</Button>
        </Row>
    </Col>
    ;
}