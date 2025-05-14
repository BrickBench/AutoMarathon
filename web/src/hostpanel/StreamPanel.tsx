import { useContext } from "react";
import { Event, Person, Runner, RunnerState, RunnerStateEntry, StreamHost, StreamScene, StreamSource } from "../websocket";
import { StreamStateContext, WebUIStateContext } from "../Context";
import Select from "react-select";
import { Button, ButtonGroup, ButtonToolbar, Col, Container, InputGroup, Ratio, Row } from "react-bootstrap";
import { customStyles } from "../Globals";
import { ArrowClockwise, DashLg, VolumeMuteFill, VolumeUpFill } from "react-bootstrap-icons";
import { removePlayerSelectLayout, swapPlayerPlayerLayout } from "./LayoutUtilities";
import { doPost } from "../Api";

function StreamElement({ host, source, edit, sourceIndex, runners, people, selectedEvent }: {
  host: StreamHost, source: StreamSource, sourceIndex: string, edit: boolean, runners: Map<number, Runner>
    , people: Map<number, Person>, selectedEvent: Event | undefined
}) {
  const { streamContext, selectedLayoutContext } = useContext(StreamStateContext);
  const [streamState, setStreamState] = streamContext;
  const [selectedLayout, setSelectedLayout] = selectedLayoutContext;

  let selectedRunner = streamState ? streamState.stream_runners[sourceIndex] : undefined;
  let selectedRunnerNum = selectedRunner ? selectedRunner : -1;
  let selectedRunnerName = selectedRunner ? people.get(selectedRunnerNum)!.name : '';

  let isAudibleRunner = streamState ? (selectedRunnerNum == streamState.audible_runner) && selectedRunner : false;


  type DragDropData = {
    id: string;
  };

  const handleDragStart = (data: DragDropData) => event => {
    let fromBox = JSON.stringify({ id: data.id });
    event.dataTransfer.setData("dragContent", fromBox);
  };

  const handleDragOver = (data: DragDropData) => event => {
    event.preventDefault();
    return false;
  };

  const handleDrop = (data: DragDropData) => event => {
    event.preventDefault();
    let fromBox = JSON.parse(event.dataTransfer.getData("dragContent"));
    let position1 = fromBox.id;
    let position2 = data.id;

    if (!streamState) return;

    let [newstate, warning, errormessage] = swapPlayerPlayerLayout(host, streamState, position1, position2)

    setStreamState(newstate);
    return false;
  };

  const width = 1920;
  const height = 1080;
  const panelStyle = {
    position: "absolute",
    top: `${source.y / height * 100}%`,
    left: `${source.x / width * 100}%`,
    width: `${source.width / width * 100}%`,
    height: `${source.height / height * 100}%`
  };

  let numOnScreen = Object.keys(host.scenes[selectedLayout].sources).length;
  let selectOptions = streamState && selectedEvent ? Array.from(Object.entries(selectedEvent.runner_state)
    .map<[string, RunnerStateEntry, boolean]>(([key, runnerEntry]) => {
      let index = Object.entries(streamState.stream_runners).findIndex(([_, val2]) => { return parseInt(key) == val2 });
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

  return (<>{(source.width > 100 && source.height > 100) &&
    <Container fluid={true}
      onDragStart={handleDragStart({ id: sourceIndex })}
      onDragOver={handleDragOver({ id: sourceIndex })}
      onDrop={handleDrop({ id: sourceIndex })} className="bg-light-subtle border border-secondary p-1" style={panelStyle} draggable={true}>
      {streamState && selectedEvent ? <>
        <Row style={{ height: "100%" }}>
          <Col lg={2}></Col>
          <Col lg={8} className="d-flex">
            <ButtonToolbar className="align-items-center justify-content-center" role="toolbar" aria-label="Toolbar with button groups">
              <InputGroup>
                {edit ? (
                  <Select styles={customStyles} value={selectedRunner ? { value: selectedRunner, label: people.get(selectedRunnerNum)!.name } : undefined}
                    options={selectOptionsPlayerChecks}
                    onChange={selectedOption => {
                      let layout_temp = { ...streamState.stream_runners };

                      let sourceStr = sourceIndex.toString();

                      delete layout_temp[sourceStr];

                      if (selectedOption) {
                        layout_temp[sourceStr] = selectedOption.value;
                      }

                      let currentSound = isAudibleRunner && selectedRunnerNum;
                      setStreamState({
                        ...streamState,
                        stream_runners: layout_temp,
                        audible_runner: (currentSound ? selectedRunnerNum : streamState.audible_runner)
                      });
                    }}
                  ></Select>) : selectedRunnerName}
              </InputGroup>
              <ButtonGroup role="group" aria-label="First group">
                {streamState.stream_runners[sourceIndex] && edit &&
                  <><Button type="button" className={
                    ((isAudibleRunner) ? "btn-success active" : "btn-secondary")
                  } onClick={() => {
                    let runner = streamState.stream_runners[sourceIndex];
                    if (runner) {
                      setStreamState({
                        ...streamState, audible_runner:
                          (runner == streamState.audible_runner) ? undefined : runner
                      });
                    }
                  }}>
                    {(isAudibleRunner && streamState.stream_runners[sourceIndex]) ? <VolumeUpFill /> : <VolumeMuteFill />}
                  </Button>
                    <Button className={((runners.get(selectedRunnerNum)!.stream_urls && (Object.entries(runners.get(selectedRunnerNum)!.stream_urls).length != 0)) ? "btn-primary" : "btn-warning")} type="button" onClick={() => {
                      doPost('runner/refresh', 'POST', {
                        id: selectedRunnerNum
                      });
                    }}>
                      <ArrowClockwise />
                    </Button></>
                }
              </ButtonGroup>
            </ButtonToolbar>
          </Col>
          <Col lg={2} className="d-flex align-items-start flex-column pe-3 pt-1">
            {edit &&
              <Button type="button" variant="danger" className="p-0 align-self-end" style={{ width: "1.5rem", height: "1.5rem" }}
                onClick={() => {
                  let [newstate, bestlayout, warning, errormessage] = removePlayerSelectLayout(host, streamState, selectedLayout, sourceIndex, selectedEvent);
                  if (errormessage) {
                    alert(errormessage);
                  } else if (warning) {
                    var result = confirm(warning + " Do you want to continue?");
                    if (result) {
                      setStreamState(newstate);
                      setSelectedLayout(bestlayout || selectedLayout);
                    }
                  } else {
                    setStreamState(newstate);
                    setSelectedLayout(bestlayout || selectedLayout);
                  }
                }}>
                <DashLg />
              </Button>}
            <h6 className="text-bold mt-auto align-self-end" style={{ fontSize: "2rem" }}>{sourceIndex}</h6>
          </Col>
        </Row>
      </> : <div className="d-flex align-items-start flex-column pe-3 pt-1">
        <h6 className="text-bold mt-auto align-self-end" style={{ fontSize: "2rem" }}>{sourceIndex}</h6>
      </div>}
    </Container>}</>
  );
}

export function StreamPanel({ host, runners, people, events }: { host: StreamHost, runners: Map<number, Runner>, people: Map<number, Person>, events: Event[] }) {
  const { streamContext, selectedLayoutContext } = useContext(StreamStateContext);
  const [streamState, setStreamState] = streamContext;
  const [selectedLayout, setSelectedLayout] = selectedLayoutContext;

  const panelStyle = {
    color: "white",
    backgroundColor: "DodgerBlue",
    fontFamily: "Arial",
    width: "100%"
  };

  let selectedScene: StreamScene = host.scenes[selectedLayout];

  let hostSources = Object.entries(selectedScene.sources);

  let selectedEvent = streamState ? events.find(element => element.id == streamState.event) : undefined;

  //Select the largest panels per source id to use for the editor display.
  let filteredSources = new Map(hostSources.map<[string, number]>(([sourceIndex, sources]) => {
    let sortedSources = (sources.map<[StreamSource, number]>((subsource, index) => [subsource, index]).sort(
      ([subsource1, subsource_index1], [subsource2, subsource_index2]) => {
        let area1 = subsource1.width * subsource1.height;
        let area2 = subsource2.width * subsource2.height;

        if (area1 < area2) {
          return 1;
        }
        if (area1 > area2) {
          return -1;
        }

        return subsource_index1 > subsource_index2 ? 1 : -1;
      }
    ).map(([_, index]) => index));

    let bestIndex: number = sortedSources.length ? sortedSources[0] : -1;
    return [sourceIndex, bestIndex];
  }));

  return (
    <Ratio aspectRatio="16x9" className="bg-body-tertiary" style={panelStyle}>
      <div>
        {selectedScene ?
          (hostSources.length > 0 ? hostSources.flatMap(([index, sources]) => {
            return sources.map<[string, StreamSource, number]>((source, subIndex) => [index, source, subIndex]);
          }).map(([index, source, subIndex]) => {
            return <StreamElement key={index + ":" + subIndex} host={host} sourceIndex={index} edit={subIndex == (filteredSources.get(index) ?? -1)}
              source={source} runners={runners} people={people} selectedEvent={selectedEvent} />;
          })
            :
            <Row className="h-100 justify-content-center align-items-center">
              <Col md={12} className="d-flex justify-content-center">
                <h1>{selectedScene.name}</h1>
              </Col>
            </Row>
          )
          :
          <Row className="h-100 justify-content-center align-items-center">
            <Col md={12} className="d-flex justify-content-center">
              <h1>No Layouts Available</h1>
            </Col>
          </Row>
        }
      </div>
    </Ratio>
  );
}
