/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/**
 * AddressResource.java
 *
 * @author Peter Halasz
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

