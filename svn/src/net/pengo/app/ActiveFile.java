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
 * ActiveFile.java
 *
 * @author Peter Halasz
 */

package net.pengo.app;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.Iterator;
import java.util.Set;

import net.pengo.data.Data;
import net.pengo.resource.ResourceFactory;
import net.pengo.restree.ResourceList;

// currently viewed file by a component or set of components.
// e.g. A shared panel in an MDI still only has one ActiveFile, but.. you get the idea

// OpenFile's cannot be shared between ActiveFile available lists

public class ActiveFile {
    OpenFile active; // currently active file
    ResourceList available = new ResourceList(new ArrayList(), (ResourceFactory)null, "Active files"); // all OpenFile's available to edit
    Set listeners = new HashSet(); //
    
    public ActiveFile() {
        super();
    }
    
    public ActiveFile(OpenFile d) {
        this();
        setActive(d, ActiveFile.class);
    }
    public ActiveFile(Data d) {
        this();
        setActive(new OpenFile(d, this), null);
    }
    
    public ResourceList getResourceList() {
        return available;
    }
    
    public void addActiveFileListener(ActiveFileListener l) {
        listeners.add(l);
    }
    
    public void removeActiveFileListener(ActiveFileListener l) {
        listeners.remove(l);
    }
    public boolean closeAll(Object source) {
        //fixme: should check that all can be closed before closing any
        for (Iterator i = available.iterator(); i.hasNext(); ) {
            OpenFile of = (OpenFile)i.next();
            
            ActiveFileEvent e = new ActiveFileEvent(source, this, of);
            for (Iterator it=listeners.iterator(); it.hasNext(); ) {
                ActiveFileListener l = (ActiveFileListener)it.next();
                if (!l.readyToCloseOpenFile(e)) {
                    //fixme: should jump to this file and warn
                    return false;
                }
            }
        }
        
        //fixme: close all does not work. (ConcurrentModificationException)
        //for (Iterator i = available.iterator(); i.hasNext(); ) {
        //    OpenFile of = (OpenFile)i.next();
		
		//fixme: should be able to use iterator. fix iterator in ResourceList!
		OpenFile[] availArray = (OpenFile[])available.toArray(new OpenFile[0]);
		for (int i=0; i<availArray.length; i++) {
			OpenFile of = availArray[i];
            boolean success = close(of, source);
            if (!success)
                return false;
            
            //i.remove(); //fixme. can't be done with ResourceList's iterator yet
			available.remove(of);
            
            ActiveFileEvent e = new ActiveFileEvent(source, this, of);
            for (Iterator it=listeners.iterator(); it.hasNext(); ) {
                ActiveFileListener l = (ActiveFileListener)it.next();
                l.closedOpenFile(e);
            }
        }
        
        //available.clear();
        
        setActive(null, source);
        return true;
    }
    
    // closes the file and remove from list
    public boolean close(OpenFile closeFile, Object source) {
        if (closeFile == null)
            return false;
        
        if (!available.contains(closeFile))
            //throw new IllegalArgumentException("file to close is not even open");
            return false;
        
        if (closeFile == active) {
            setActive(null, source);
        }
        
        ActiveFileEvent e = new ActiveFileEvent(source, this, active);
        for (Iterator i=listeners.iterator(); i.hasNext(); ) {
            ActiveFileListener l = (ActiveFileListener)i.next();
            if (!l.readyToCloseOpenFile(e)) {
                System.out.println("not ready to close");
                return false;
            }
        }
        
        //closeFile.setActiveFile(null);
        boolean done = closeFile.close(source);
        
        if (!done) {
            System.out.println("failed to close");
            return false;
        }
        
        available.remove(closeFile);
        
        for (Iterator i=listeners.iterator(); i.hasNext(); ) {
            ActiveFileListener l = (ActiveFileListener)i.next();
            l.closedOpenFile(e);
        }
        
        return true;
    }
    
    // sets "active" as the active OpenFile. If not in the available list, it is added.
    public void setActive(OpenFile active, Object source) {
        if (active != null && !available.contains(active)) {
            available.add(active);
            active.setActiveFile(this);
        }
        
        this.active = active;
        
        ActiveFileEvent e = new ActiveFileEvent(source, this, active);
        for (Iterator i=listeners.iterator(); i.hasNext(); ) {
            ActiveFileListener l = (ActiveFileListener)i.next();
            l.activeChanged(e);
        }
    }
    
    
    // may return null
    public OpenFile getActive() {
        return active;
    }
    
}
