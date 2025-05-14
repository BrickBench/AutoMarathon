import { Button, ButtonGroup, ButtonToolbar, Col, Row } from "react-bootstrap";
import { StreamStateContext, WebUIStateContext } from "../Context";
import { useContext, useEffect, useState } from "react";
import { HostEventSelector, LayoutSelector, PlayerSelector, TransitionSelector } from "./SidePanel";
import { StreamPanel } from "./StreamPanel";
import { StreamEntry, StreamHost, Event, Person, Runner } from "../websocket";
import { getSelectedLayout, getStreamForHost, updateStreamRequest } from "./LayoutUtilities";
import { ToastNotifStateContext } from "../AMNotification";

export function HostPanel({ host, events, people, streams, runners }: {
  host: StreamHost, events: Event[], people: Map<number, Person>, streams: StreamEntry[],
  runners: Map<number, Runner>
}) {
  const { webuistate, setWebUIState } = useContext(WebUIStateContext);
  const [selectedLayoutState, setSelectedLayoutState] = useState<string>(getSelectedLayout(host));
  const [streamState, setStreamState] = useState(getStreamForHost(streams, webuistate.selectedHost))
  const { toastNotifState, setToastNotifState } = useContext(ToastNotifStateContext);
  //const {notification, setNotification } = useContext(NotificationContext);

  useEffect(() => {
    setStreamState(getStreamForHost(streams, webuistate.selectedHost));
  }, [streams])

  let streamEvent = streamState ? events.find(element => element.id == streamState.event) : undefined;

  return (
    <StreamStateContext.Provider value={{
      selectedLayoutContext: [selectedLayoutState, setSelectedLayoutState],
      streamContext: [streamState, setStreamState]
    }}>
      <Row>
        <Col lg={9}>
          <StreamPanel host={host} runners={runners} people={people} events={events}></StreamPanel>
        </Col>
        <Col lg={3}>
          <Row>
            <HostEventSelector events={events} host={host}></HostEventSelector>
          </Row>
          <Row>
            {streamEvent ? <PlayerSelector host={host} people={people} event={streamEvent}></PlayerSelector> : <></>}
          </Row>
          <LayoutSelector host={host}></LayoutSelector>
          <TransitionSelector></TransitionSelector>
          {streamState ?
            <ButtonToolbar className="mt-4" role="toolbar" aria-label="Toolbar with button groups">
              <ButtonGroup>
                <Button variant="danger" onClick={() => {
                  setStreamState(getStreamForHost(streams, webuistate.selectedHost));
                  setSelectedLayoutState(getSelectedLayout(host));
                }}>Revert Changes</Button>
              </ButtonGroup>
              <ButtonGroup>
                <Button variant="primary" onClick={() => {
                  let prompt = false;
                  let hasAudible = false;

                  let numOnScreen = Object.keys(host.scenes[selectedLayoutState].sources).length;
                  let found = new Set<number>();
                  for (let [key, val] of Object.entries(streamState.stream_runners)) {
                    if ((parseInt(key) < numOnScreen + 1) && val == streamState.audible_runner) {
                      hasAudible = true;
                    }
                    if ((parseInt(key) < numOnScreen + 1) && found.has(val)) {
                      prompt = true;
                    } else if (parseInt(key) < numOnScreen + 1) {
                      found.add(val);
                    }
                  }
                  var continueOn = true;
                  if (prompt) {
                    alert("Your layout contains two of the same runner. Please remove duplicate runners.");
                    return;
                  }
                  //Check Audible Runners
                  if (!hasAudible && numOnScreen != 0) {
                    continueOn = confirm("No onscreen runner has audio selected. Are you sure you want to have no runners on screen with audio?");
                  }

                  if (numOnScreen == 0) {
                    delete streamState.audible_runner;
                  }

                  if (continueOn) {
                    let previousStream = streams.find(element => element.obs_host == webuistate.selectedHost);
                    const previousEvent = previousStream ? previousStream.event : undefined;
                    updateStreamRequest(previousEvent, { ...streamState, requested_layout: selectedLayoutState }, (successful) => {
                      if (successful) {
                        setToastNotifState({ notification: "Wait a couple seconds for AutoMarathon to edit the layout. Redo the change if AutoMarathon fails to make the change within 15 seconds.", toggle: true, success: true });
                      } else {
                        setToastNotifState({ notification: "Try reverting your changes and reattempting after 10 seconds.", toggle: true, success: false });
                      }

                    });
                  }
                }}>Push Changes</Button>
              </ButtonGroup>
            </ButtonToolbar>
            : <></>}
        </Col>
      </Row>
      <Row className="border-top pt-3">
        <Col lg={6}>
        </Col>
        <Col lg={6}>
        </Col>
      </Row>
    </StreamStateContext.Provider>);
}
