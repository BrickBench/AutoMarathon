import { useContext, useEffect, useRef, useState } from "react";
import { StreamStateContext, WebUIStateContext } from "../Context";
import { StreamHost, StreamScene, Event, Person, Runner } from "../websocket";
import Select from "react-select";
import { Button, Col, InputGroup, Row } from "react-bootstrap";
import { PlusLg } from "react-bootstrap-icons";
import { customStyles } from "../Globals";
import { addPlayerSelectLayout, autoFillLayout } from "./LayoutUtilities";
import { doPost } from "../Api";

export function PlayerSelector({ host, people, event }: { host: StreamHost, people: Map<number, Person>, event: Event }) {
    const { streamContext, selectedLayoutContext } = useContext(StreamStateContext);
    const [streamState, setStreamState] = streamContext;
    const [selectedLayout, setSelectedLayout] = selectedLayoutContext;

    if (!streamState) {
        return;
    }

    let numOnScreen = Object.keys(host.scenes[selectedLayout].sources).length;
    let selectOptions = Array.from(Object.entries(event.runner_state)
        .filter(([key, val]) => {
            let index = Object.entries(streamState.stream_runners).findIndex(([key2, val2]) => { return parseInt(key) == val2 });
            return index == -1 || index >= numOnScreen;
        })
        , ([key, value]) => ({ value: parseInt(key), label: people.get(parseInt(key))!.name }));

    const [selectedPlayerState, setSelectedPlayerState] = useState<{ value: number, label: string } | undefined>(selectOptions[0]);

    useEffect(() => {
        if (selectOptions.length > 0) setSelectedPlayerState(selectOptions[0]);
    }, [event, streamState]);

    return (
        <Col className="mt-3">
            <Row>
                <h4>Add Player to Stream</h4>
            </Row>
            <Row>
                <InputGroup>
                    <div className="m-0 p-0">
                        <Select value={selectedPlayerState} styles={customStyles}
                            options={selectOptions} onChange={(option) => setSelectedPlayerState(option ?? undefined)}></Select>
                    </div>
                    <div className="input-group-append">
                        <Button variant="success" onClick={() => {
                            if (!selectedPlayerState) {
                                return;
                            }

                            let addedPlayer = selectedPlayerState.value;

                            let [newstate, bestlayout, warning, errormessage] = addPlayerSelectLayout(host, streamState, selectedLayout, addedPlayer, event);

                            if (errormessage) {
                                alert(errormessage);
                            } else if (warning) {
                                var result = confirm(warning + " Do you want to continue?");
                                if (result) {
                                    setStreamState(newstate);
                                    setSelectedLayout(bestlayout);
                                }
                            } else {
                                setStreamState(newstate);
                                setSelectedLayout(bestlayout);
                            }
                        }}><PlusLg /></Button>
                    </div>
                </InputGroup>
            </Row>
        </Col>);
}

export function HostEventSelector({ events, host }: { events: Event[], host: StreamHost }) {
    const { webuistate, setWebUIState } = useContext(WebUIStateContext);
    const { streamContext, selectedLayoutContext } = useContext(StreamStateContext);
    const [streamState, setStreamState] = streamContext;
    const [selectedLayout, setSelectedLayout] = selectedLayoutContext;

    let streamEventID = (streamState || { event: -1 }).event;

    let streamEvent = events.find(element => element.id == streamEventID);
    let selectedEvent = events.find(element => element.id == webuistate.selectedEvent)!;

    return (
        <div className="mt-3">
            {streamState ?
                <>
                    <h4>Event: <b>{streamEvent.name}</b></h4>
                    {(streamEventID != webuistate.selectedEvent) &&
                        <Button variant="primary" onClick={() => {
                            let tempStream = { ...streamState };
                            tempStream.event = webuistate.selectedEvent;

                            let [new_layout, stream_map] = autoFillLayout(selectedEvent, host);
                            tempStream.stream_runners = stream_map;

                            setStreamState(tempStream);
                            setSelectedLayout(new_layout ? new_layout : selectedLayout);
                        }}>Change To <b>{selectedEvent!.name}</b></Button>}
                    <Button variant="danger" onClick={() => {
                        doPost('stream', 'DELETE', {
                            id: streamState.event
                        });
                    }}>Unassign Event</Button>
                </> :
                <>
                    <Button variant="primary" onClick={() => {
                        doPost('stream', 'POST',
                            {
                                event: webuistate.selectedEvent,
                                obs_host: webuistate.selectedHost,
                                active_commentators: '',
                                ignored_commentators: '',
                                stream_runners: {}
                            });
                    }}>Set Host to <b>{selectedEvent!.name}</b></Button>
                </>
            }
        </div>
    );
}

export function LayoutSelector({ host }: { host: StreamHost }) {
    const { streamContext, selectedLayoutContext } = useContext(StreamStateContext);
    const [streamState, setStreamState] = streamContext;
    const [selectedLayout, setSelectedLayout] = selectedLayoutContext;

    let layoutComparator = ([scene1Name, scene1]: [string, StreamScene], [scene2Name, scene2]: [string, StreamScene]) => {
        let numScene1Sources = Object.keys(scene1.sources).length;
        let numScene2Sources = Object.keys(scene2.sources).length;
        if (numScene1Sources == 0 && numScene2Sources != 0) {
            return 1;
        } else if (numScene1Sources != 0 && numScene2Sources == 0) {
            return -1;
        } else if (numScene1Sources > numScene2Sources) {
            return 1;
        } else if (numScene1Sources < numScene2Sources) {
            return -1;
        } else {
            return scene1Name.localeCompare(scene2Name);
        }
    };

    return (
        <div className="mt-3">
            <h4>Layouts</h4>
            <Select styles={customStyles} value={{ value: selectedLayout, label: selectedLayout }}
                options={Array.from(Object.entries(host.scenes).sort(layoutComparator), ([key, _]) => ({ value: key, label: key }))}
                onChange={selectedOption => {
                    if (selectedOption) {
                        setSelectedLayout(selectedOption.value);
                    }
                }}></Select>
        </div>
    );
}

export function TransitionSelector() {
    return <></>;
    /*return (
      <div className="mt-3">
        <h4>Transition</h4>
        <select className="form-select">
          <option value="1" selected>Wipe (Layout Default)</option>
          <option value="2">Fade</option>
          <option value="3">Circle</option>
        </select>
      </div>
    );*/
}
