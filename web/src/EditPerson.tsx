import { Event, Person, Runner } from './websocket';
import { useState, useContext, useEffect } from 'react'
import { Form, Button, Row, Col, FormControl, FormLabel, InputGroup } from 'react-bootstrap';
import { ArrowClockwise, Dash, DashLg, Discord, Twitch, Twitter, Youtube } from 'react-bootstrap-icons';
import { WebUIStateContext } from './Context';
import { doPost } from './Api'
import InputGroupText from 'react-bootstrap/esm/InputGroupText';

function runnerInEvent(runnerID: number, events: Event[]) {
  for (let event of events) {
    if (event.runner_state[runnerID.toString()]) {
      return event.name;
    }
  }
  return undefined;
}

export function EditPerson({ person, runner, events }: { person: Person | undefined, runner: Runner | undefined, events: Event[] }) {
  const { webuistate, setWebUIState } = useContext(WebUIStateContext);
  const [personEditState, setPersonEditState] = useState<Person | undefined>(person);
  const [runnerEditState, setRunnerEditState] = useState<Runner | undefined>(runner);
  const [runnerEnabledState, setRunnerEnabledState] = useState<boolean>(!(runner === undefined));
  const [previousRunnerState, setPreviousRunnerState] = useState<boolean>(!(runner === undefined));

  useEffect(() => {
    setPersonEditState(person);
    setRunnerEditState(runner);
    setRunnerEnabledState(!(runner === undefined));
    setPreviousRunnerState(!(runner === undefined));
  }, [person, runner]);

  return personEditState && person && <>
    <div className="d-flex justify-content-between pt-2">
      <div>
        <h2>Edit {runnerEnabledState ? "Runner" : "Person"}</h2>
      </div>
      <div>
        <Button variant="danger" onClick={() => {
          var result = confirm("Are you sure you want to delete this person?");
          if (result) {
            if (previousRunnerState && runner) {
              doPost('runner', 'DELETE', {
                id: runner.participant
              });
            }
            doPost('participant', 'DELETE', {
              id: person.id
            });
            setWebUIState({ ...webuistate, selectedPerson: -1 });
          }
        }}>Delete Person</Button>
      </div>
    </div>
    <Form onSubmit={(event) => {
      event.preventDefault();
      if (previousRunnerState && !runnerEnabledState && runnerEditState) {
        doPost('runner', 'DELETE', {
          id: runnerEditState.participant
        });
      } else if (!previousRunnerState && runnerEnabledState && runnerEditState) {
        doPost('runner', 'POST', { ...runnerEditState, stream_urls: {} });
      } else if (previousRunnerState == runnerEnabledState && runnerEditState) {
        doPost('runner', 'PUT', runnerEditState);
      }
      doPost('participant', 'PUT', personEditState);

    }}>
      <Row className="mb-3">
        <Col>
          <FormLabel htmlFor="personeditname">Name</FormLabel>
          <FormControl type="text" id="personeditname" onChange={e => setPersonEditState({
            ...personEditState,
            name: e.target.value
          })} value={personEditState.name || ''} />
        </Col>
        <Col>
          <FormLabel htmlFor="personeditpronouns">Pronouns</FormLabel>
          <FormControl type="text" id="personeditpronouns" onChange={e => setPersonEditState({
            ...personEditState,
            pronouns: e.target.value
          })} value={personEditState.pronouns || ''} />
        </Col>
        <Col>
          <FormLabel htmlFor="runnereditlocation">ISO 3166-2 Location</FormLabel>
          <FormControl type="text" id="runnereditlocation" onChange={e => setPersonEditState({
            ...personEditState,
            location: e.target.value
          })} value={personEditState.location || ''} />
        </Col>
      </Row>
      <h3>Socials</h3>
      <Row className="mb-3">
        <Col>
          <FormLabel htmlFor="runneredittwitchsocial"><Twitch /> Twitch</FormLabel>
          <FormControl type="text" id="runneredittwitchsocial" />
        </Col>
        <Col>
          <FormLabel htmlFor="runneredittwittersocial"><Twitter /> Twitter</FormLabel>
          <FormControl type="text" id="runneredittwittersocial" />
        </Col>
        <Col>
          <FormLabel htmlFor="runneredityoutubesocial"><Youtube /> Youtube</FormLabel>
          <FormControl type="text" id="runneredityoutubesocial" />
        </Col>
        <Col>
          <FormLabel htmlFor="runnereditdiscordid"><Discord /> Discord ID</FormLabel>
          <FormControl type="text" id="runnereditdiscordid" onChange={e => setPersonEditState({
            ...personEditState,
            discord_id: e.target.value
          })} value={personEditState.discord_id || ''} />
        </Col>
      </Row>
      <Row>
        <Col>
          <h3>Runner Info</h3>
        </Col>
        <Col>
          <FormLabel htmlFor="personisrunner">Is Runner</FormLabel>
          <Form.Check type="switch" id="personisrunner" checked={runnerEnabledState}
            onChange={e => {
              //Don't allow deleting the runner if they are in a event.
              if (runnerEditState && !e.target.checked) {
                let runEvent = runnerInEvent(runnerEditState.participant, events);
                if (!runEvent) {
                  setRunnerEnabledState(false);
                  return;
                } else {
                  alert("Runner is in an event: " + runEvent + ". Remove them from all events first before removing them as a runner.");
                  setRunnerEnabledState(true);
                  return;
                }
              }
              if (runnerEditState === undefined && e.target.checked) {
                setRunnerEditState({ participant: person.id, stream_volume_percent: 100 });
              }
              setRunnerEnabledState(e.target.checked);
            }} />
        </Col>
      </Row>
      {(runnerEnabledState && runnerEditState) && <>
        <Row className="pt-3">
          <Col lg={5}>
            <FormLabel htmlFor="runnereditstream">Stream Link</FormLabel>
            <FormControl type="text" id="runnereditstream" onChange={e => setRunnerEditState({
              ...runnerEditState,
              stream: e.target.value
            })} value={runnerEditState.stream || ''} />
          </Col>
          <Col lg={5}>
            <Row className="mb-3">
              <FormLabel htmlFor="runneroverideurl">StreamLink Override</FormLabel>
              <FormControl type="text" placeholder="m3u8 link" aria-label="runneroverideurl" onChange={e => setRunnerEditState({
                ...runnerEditState,
                override_stream_url: e.target.value
              })} value={runnerEditState.override_stream_url || ''} />
            </Row>
          </Col>
          <Col lg={2}>
            <Button variant={((runnerEditState.stream_urls && (Object.keys(runnerEditState.stream_urls).length != 0)) ? "primary" : "warning")} type="button" id="button-refreshlink" onClick={() => {
              doPost('runner/refresh', 'POST', {
                id: runnerEditState.participant
              });
            }}>Refresh Streamlink<ArrowClockwise /></Button>
          </Col>
        </Row>
        <Row className="pt-3">
          <Col lg={6}>
            <FormLabel htmlFor="runnervolume" className="form-label">Volume</FormLabel>
            <Row>
              <Col lg={9}>
                <Form.Range min={-100} max={0} value={-runnerEditState.stream_volume_percent || 0} onChange={e => setRunnerEditState({
                ...runnerEditState,
                stream_volume_percent: Math.max(-parseInt(e.target.value),0)
              })}/>
              </Col>
              <Col lg={3}>
                <InputGroup>
                  <InputGroupText><DashLg></DashLg></InputGroupText>
                  <FormControl type="text" className="form-control" id="runnereditvolume" onChange={e => setRunnerEditState({
                  ...runnerEditState,
                  stream_volume_percent: Math.max(Math.min(parseInt(e.target.value),100),0)
                })} value={runnerEditState.stream_volume_percent || 0} />
                  <InputGroupText>dB</InputGroupText>
                </InputGroup>
              </Col>
            </Row>
          </Col>
          <Col lg={6}>
            <FormLabel htmlFor="runneredittherun" className="form-label">therun.gg Username</FormLabel>
            <FormControl type="text" className="form-control" id="runneredittherun" onChange={e => setRunnerEditState({
              ...runnerEditState,
              therun: e.target.value
            })} value={runnerEditState.therun || ''} />
          </Col>
        </Row>
      </>
      }
      <Button type="submit" variant="primary">Save Changes</Button>
    </Form>
  </>;
}
