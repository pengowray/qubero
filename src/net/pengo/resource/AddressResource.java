/**
 * AddressResource.java
 *
 * @author Created by Omnicore CodeGuide
 *
 * A restricted SelectionResource, which can only contain a start and length.
 * However it doesn't use a SimpleLongListSelectionModel and is pretty useless.
 * I dont think it's used by anything.
 */

package net.pengo.resource;

import net.pengo.app.OpenFile;
import net.pengo.data.Data;
import net.pengo.data.SelectionData;
import net.pengo.selection.LongListSelectionModel;
import net.pengo.selection.SimpleLongListSelectionModel;

public class AddressResource extends SelectionResource
{
    
    private IntResource address;
    private IntResource length;

    //private LongListSelectionModel sel;
    private SelectionData selData; // cache thing
    
	public Resource[] getSources()
	{
		return new Resource[]{address, length};
	}

    
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
    
    public void setSelection(LongListSelectionModel sel){
    	//FIXME: pending.
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
    	//sel = null;
        selData = null;
    }
    
    
    //private List dependantsList; //fixme: pending
    //public void dependencyChanged(Resource changed) {}

    
    public String descValue() {
        //return "Pointer@" + Long.toString(address.getSelectionResource().getSelectionData().getStart(), 16) +" -> address:" + address.getValue().toString(16) + " = " + toSelection();
        //return address.getValue().toString(16) + " = " + getSelection();
        return getSelection().toString();
    }


	/* (non-Javadoc)
	 * @see net.pengo.resource.SelectionResource#insertReplaceResize(net.pengo.data.Data)
	 */
	public void insertReplaceResize(Data newdata) {
		
		
	}


}

