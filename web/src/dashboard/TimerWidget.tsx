import { useEffect, useState } from "react";
import { Event } from "../websocket";
import { Button, ButtonGroup, ButtonToolbar, Card, CardBody, CardHeader, CardTitle, FormControl, InputGroup } from "react-bootstrap";
import { PauseFill, PlayFill, X } from "react-bootstrap-icons";
import { doPost } from "../Api";

function displayTimer(startTime, endTime, pauseTime){
  let millisElapsed = 0;
  if(startTime){
    if(endTime){
      millisElapsed = endTime - startTime;
    } else {
      millisElapsed = Date.now() - startTime;
    }
  }

  if(pauseTime){
    millisElapsed -= pauseTime;
  }

  let hours = Math.floor((millisElapsed / (1000 * 60 * 60)) % 24);
  let minutes = Math.floor((millisElapsed / 1000 / 60) % 60);
  let seconds = Math.floor((millisElapsed / 1000) % 60);
  let millis = millisElapsed % 1000;

  let hoursS = (hours < 10) ? "0" + hours : hours;
  let minutesS = (minutes < 10) ? "0" + minutes : minutes;
  let secondsS = (seconds < 10) ? "0" + seconds : seconds;
  let millisS = millis.toString().padStart(3, "0").substring(0,2);
  
  return hoursS + ":" + minutesS + ":" + secondsS + "." + millisS;
}
  
export function TimerWidget({event} : {event : Event | undefined}){

    const [startTime, setStartTime] = useState<number | null>(event?.timer_start_time ?? null);
    const [endTime, setEndTime] = useState<number | null>(event?.timer_end_time ?? null);
    const [pauseTime, setPauseTime] = useState<number | null>(0);
    const [timerString, setTimerString] = useState<string>(displayTimer(startTime, endTime, pauseTime));
    const [editTimerString, setEditTimerString] = useState<string>("");

    useEffect(()=>{
      setStartTime(event?.timer_start_time ?? null);
      setEndTime(event?.timer_end_time ?? null);
      setPauseTime(0);
    },[event]);

    useEffect(()=>{
      const interval = setInterval(() => {
        setTimerString(displayTimer(startTime, endTime, pauseTime));
      }, 50);

      return () => clearInterval(interval);
    });

    let paused = startTime && endTime;

    return (
    <Card>
        <CardHeader className={event ? "" : "bg-danger"}>{event ? "Timer" : "Assign Host Before Using Timer"}</CardHeader>
        <CardBody>
          <CardTitle>
            <h3>{timerString}</h3>
          </CardTitle>
          <ButtonToolbar className="mb-3" role="toolbar" aria-label="Toolbar with button groups">
            <ButtonGroup className="me-2" role="group" aria-label="First group">
                <Button variant={paused || !startTime ? "primary" : "secondary"} disabled={!event} onClick={(e)=>{
                    let newStartTime = startTime;
                    let newEndTime = endTime;
                    let newPauseTime = pauseTime;
                    if(!startTime){
                      newStartTime = Date.now();
                      setStartTime(newStartTime);
                    } else {
                      if(endTime){
                        newPauseTime = (pauseTime ?? 0) + (Date.now()-endTime);
                        setPauseTime(newPauseTime);
                        newEndTime = null;
                        setEndTime(newEndTime);
                      }else{
                        newEndTime = Date.now()
                        setEndTime(newEndTime);
                      }
                    }
                    console.log({...event, timer_start_time: newStartTime,
                      timer_end_time: newEndTime
                    })
                    doPost('event','PUT', {...event, timer_start_time: newStartTime,
                      timer_end_time: newEndTime
                    }); 
                  }
                }>
                  {!startTime || (startTime && endTime) ? <PlayFill/> : <PauseFill/>}
                </Button>
                <Button variant="danger" disabled={!event} onClick={(e)=>{
                  if(confirm("Are you sure you want to reset")){
                    setStartTime(null);
                    setEndTime(null);
                    setPauseTime(null);
                    doPost('event','PUT', {...event, timer_start_time: null,
                      timer_end_time: null
                    }); 
                  }
                }}>Reset</Button>
            </ButtonGroup>
            <InputGroup>
                <FormControl value={editTimerString} onChange={(e)=>{
                  setEditTimerString(e.currentTarget.value);
                }}/>
                <Button disabled={!event} onClick={()=>{
                  let time = editTimerString;
                  alert("Invalid time format. Please input the new time in the format HH:MM:SS.xxx.")
                }}>Set Time</Button>
            </InputGroup>
            
          </ButtonToolbar>
        </CardBody>
     </Card>
    );
  }
