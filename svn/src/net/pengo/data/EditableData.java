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
 * EditableData.java
 *
 * Created on 7 September 2002, 09:02
 */

/**
 *
 * @author  Peter Halasz
 */
package net.pengo.data;

import net.pengo.selection.*;
import java.util.*;

public abstract class EditableData extends Data {
    Set datalisteners = new HashSet() ; //DataListener's
    
    public void addDataListener(DataListener l) {
	datalisteners.add(l);
    }
    
    public void removeDataListener(DataListener l) {
	datalisteners.remove(l);
    }
    
    protected void fireDataUpdated(Object source, DiffData.Mod mod){
	for (Iterator it = datalisteners.iterator(); it.hasNext(); ) {
	    DataListener l = (DataListener)it.next();
	    DataEvent e = new DataEvent(source, mod);
	    l.dataUpdate(e);
	}
    }
    
    public abstract void delete(long offset, long length);
    
    public abstract void delete(LongListSelectionModel selection);
    
    public void insert(long offset, Data data) {
        data = data.shiftStart(offset - data.getStart());
        insert(data);
    }

    public abstract void insert(Data data);
    
    public void overwrite(long offset, Data data) {
        data = data.shiftStart(offset - data.getStart());
        overwrite(data);
    }

    public abstract void overwrite(Data data);
    
    /**
     * inserts data, replacing whatever is contained in <code>selection</code>.
     * 
     * @param selection selection to replace
     * @param newData data to replace selection with
     * @return selection fitting the new data
     */
    public LongListSelectionModel insertReplace(LongListSelectionModel selection, Data newData) {
		
		//use overwrite if possible
		if (selection.getSegmentCount() == 1 &&
		    newData.getLength() == selection.getMaxSelectionIndex() - selection.getMinSelectionIndex() +1) {
		    //fixme: if newData is has holes, this may stuff up
		    overwrite(selection.getMinSelectionIndex(), newData);
		    return selection;
		}
		
		
        delete(selection);
        //FIXME: multiple selection insertion?
        //insert(selection.getAnchorSelectionIndex(), newData); // this seems dangerous
        insert(selection.getMinSelectionIndex(), newData);
        long start = selection.getMinSelectionIndex();
        return new SimpleLongListSelectionModel(start,start+newData.getLength()-1);
    }
}
