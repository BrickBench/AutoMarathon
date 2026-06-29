import { useEffect, useState } from "react";
import { doPost } from "../Api";
import { CustomFields, Person } from "../websocket";
import { Button, FormControl, InputGroup } from "react-bootstrap";
import { customStyles } from "../Globals";
import Select from 'react-select';

function CustomFieldInput({customTemp,key2,setCustomTemp, people}: {customTemp: any,key2:any,setCustomTemp:any, people : Map<number, Person>}){
    const [inputState,setInputState]  = useState(customTemp[key2]);
    useEffect(() => {
        setInputState(customTemp[key2]);
    }, [customTemp]);


    var inputElement;

    if(key2.endsWith(":bool")){
        inputElement = <><input type="checkbox" id={key2} className="btn-check" onChange={({ target }) => {
          setInputState(String(target.checked));
          let temp = customTemp;
          temp[key2] = String(target.checked);
          setCustomTemp(temp);
        }} checked={inputState === 'true'} />
        <label className="btn btn-outline-primary" htmlFor={key2}>{String(inputState === "true")}</label>
        </>;
    }else if(key2.endsWith(":date")){
        inputElement = <FormControl type="datetime-local" onChange={e => {
            if(e.target.value){
                let date_entry = new Date(e.target.value);
                let utc = date_entry.getTime();
                setInputState(String(utc));
                let temp = customTemp;
                temp[key2] = String(utc);
                setCustomTemp(temp);
            }
        }} value={inputState && isFinite(inputState)? (new Date(new Date(parseInt(inputState)).getTime() - new Date(parseInt(inputState)).getTimezoneOffset() * 60000).toISOString()).slice(0, -1) : ''}/>
    }else if(key2.endsWith(":person")){
        let commentatorOptions =  [...people.entries()].map(([key, value]) => ({value: parseInt(value.id), label: value.name}));
        inputElement = <Select styles={customStyles} value={commentatorOptions.find((e)=> e.value == parseInt(inputState))}
        onChange={selectedOptions => {
                    console.log("ghgh",selectedOptions);
          setInputState(String(selectedOptions.value));
          let temp = customTemp;
          temp[key2] = String(selectedOptions.value);
          setCustomTemp(temp);
        }}
        options={commentatorOptions} isMulti={false}></Select>;
    }else{
        inputElement = <input type="text" name={key2} className="form-control" onChange={({ target }) => {
          setInputState(target.value);
          let temp = customTemp;
          temp[key2] = target.value;
          setCustomTemp(temp);
        }} value={inputState || ''}/>
    }

    return <>
      <label>{key2}</label>
      {inputElement}
      <Button variant={"danger"} onClick={()=>{
            if(prompt(`Please type "delete" to confirm this action`) == "delete"){
                doPost('custom-field','DELETE',{key:key2});
            }
        }}>Delete</Button>
    </>
    ;
}

export function CustomFieldWidget({customFields, people} : {customFields : CustomFields, people : Map<number, Person>}){
  const [customTemp,setCustomTemp] = useState(customFields);
  const [newField,setNewField] = useState("");

  useEffect(() => {
    setCustomTemp(customFields);
}, [customFields])


  let fields = Object.entries(customTemp);
        fields.sort(function(a, b){
        if(a[0].endsWith(":bool") && !b[0].endsWith(":bool")){
            return -1;
        }

        if(!a[0].endsWith(":bool") && b[0].endsWith(":bool")){
            return 1;
        }

        return a[0] > b[0] ? 1 : -1;
  });

  return (
    <div className="card">
      <div className="card-header">Custom Fields</div>
      <div className="card-body">
        <ul className="list-group list-group-flush">       
            {fields.map(([key,val])=>{
              const keystore = key;
              return (<li key={key} className="list-group-item">
                <CustomFieldInput key2={key} customTemp={customTemp} setCustomTemp={setCustomTemp} people={people}></CustomFieldInput>
              </li>
                )
            })}
            <li className="list-group-item">
                <InputGroup>
                    <FormControl value={newField} onChange={(e)=>{
                        setNewField(e.currentTarget.value);
                    }}>
                    </FormControl>
                    <Button variant={"success"} onClick={()=>{
                        if(newField && newField.length > 0){
                            doPost('custom-field','PUT',{key:newField,value:""});
                        }else{
                            alert("Enter nonempty field name");
                        }
                    }}>
                        Add New Field
                    </Button>
                </InputGroup>
            </li>
        </ul>
        <button className="btn btn-primary" onClick={() => {
            Object.entries(customTemp).forEach(([key,val])=>{doPost('custom-field','PUT',{key:key,value:val});});
      }}>Save Changes</button>
      </div>
    </div>
  );
}