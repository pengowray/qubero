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

/*
 * SelectionResource.java
 *
 * Created on 4 September 2002, 19:40
 */

/**
 *
 * @author  Peter Halasz
 */
package net.pengo.resource;

import net.pengo.app.OpenFile;
import net.pengo.data.Data;
import net.pengo.data.SelectionData;
import net.pengo.selection.LongListSelectionEvent;
import net.pengo.selection.LongListSelectionListener;
import net.pengo.selection.LongListSelectionModel;
import net.pengo.selection.SegmentalLongListSelectionModel;

//acts as a wrapper for LongListSelectionModel.

abstract public class SelectionResource extends Resource implements LongListSelectionListener {
    
    protected OpenFile openFile;
    
    abstract public LongListSelectionModel getSelection();
    abstract public void setSelection(LongListSelectionModel sel);
    
    abstract public SelectionData getSelectionData();
    
    public SelectionResource(OpenFile openFile){
	this.openFile = openFile;
    }
    
    public OpenFile getOpenFile(){
	return openFile;
    }
    
    /**
     * Called whenever the value of the selection changes.
     * @param e the event that characterizes the change.
     *
     */
    public void valueChanged(LongListSelectionEvent e) {
	if (e.getValueIsAdjusting()) {
	    return;
	}
	
	updated();
    }
    
    abstract public void updated();
    
    
    public String valueDesc() {
	return getSelection().toString();
    }
    
    /**
     * Replaces the data in this with new data, inserting or deleting if neccessary.
     * Does not change the selection size (see insertReplace)
     * @param newdata
     */
    public void insertReplace(Data newdata) {
	openFile.getEditableData().insertReplace(getSelection(), newdata);
    }
    
    /** replaces the data, and modifies this selection to fit the new data's size */
    public void insertReplaceResize(Data newdata) {
	LongListSelectionModel llsm = openFile.getEditableData().insertReplace(getSelection(), newdata);
	setSelection(llsm);
    }
    
    //public void editProperties() {
	// TODO Auto-generated method stub
	//System.out.println("editing... not really");
    //}
    
    /** makes the selection active (selected) in openfile. */
    public void makeActive() {
        //fixme: should check if selection already supports segments, if so just clone it instead.
	LongListSelectionModel selection = new SegmentalLongListSelectionModel( getSelection() );
	openFile.setSelectionModel(selection);
    }

}
