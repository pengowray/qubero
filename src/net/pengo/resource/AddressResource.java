/**
 * AddressResource.java
 *
 * @author Created by Omnicore CodeGuide
 *
 * a restricted sort of SelectionResource
 */

package net.pengo.resource;

import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.data.*;


import java.util.*;
import net.pengo.dependency.QNode;

public class AddressResource extends SelectionResource
{
	

	public QNode[] getSources()
	{
		return new QNode[]{address, length};
	}
	
    private IntResource address;
    private IntResource length;

    private LongListSelectionModel sel;
    private SelectionData selData; // cache thing
    
    AddressResource(OpenFile of, IntResource address, IntResource length) {
        super(of);
        this.address = address;
        this.length = length;
        
        //address.addDependent(this);
        //length.addDependent(this);
    }
    
    public void setAddress(IntResource startAddress) {
	this.address = startAddress;
	updated();
    }

    public void setLength(IntResource length) {
	this.length = length;
	updated();
    }
    
    public IntResource getAddress() {
	return address;
    }
    
    public IntResource getLength() {
	return length;
    }
    
    public LongListSelectionModel getSelection() {
        long firstIndex = address.toLong();
        long lastIndex = firstIndex + length.toLong() -1;
        return new SimpleLongListSelectionModel(firstIndex, lastIndex);
    }

    public SelectionData getSelectionData() {
        if (selData == null)
            selData = new SelectionData(getSelection(), openFile.getData());
            
        return selData;
    }

    
    public void updated() {
	sel = null;
        selData = null;
    }
    
    
    //private List dependantsList; //fixme: pending
    //public void dependencyChanged(Resource changed) {}

    

    
    public String toString() {
        //return "Pointer@" + Long.toString(address.getSelectionResource().getSelectionData().getStart(), 16) +" -> address:" + address.getValue().toString(16) + " = " + toSelection();
	return "Pointer -> address:" + address.getValue().toString(16) + " = " + getSelection();
    }
}

