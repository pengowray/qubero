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
 * ListResource.java
 *
 * Created on 17 September 2004, 05:27
 */

package net.pengo.resource;

import java.awt.event.ActionEvent;
import java.util.Iterator;
import javax.swing.AbstractAction;
import javax.swing.JMenu;
import javax.swing.JSeparator;

import net.pengo.listEditor.ListEditor;
import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.SmartPointer;

/**
 *
 * @author Peter Halasz
 */
public abstract class ListResource extends Resource { // extends DefinitionResource
    public final SmartPointer type = new JavaPointer(net.pengo.resource.TypeResource.class);    
    
    public ListResource() {
	//fixme: should use something like "Raw Hex". 
	this(new JavaType(IntAddressedResource.class));
    }
    
    public ListResource(TypeResource type) {
	this.type.setName("Type");
	this.type.addSink(this);
	this.type.setValue(type);
    }
    
    public abstract void setValue(Resource[] array);
    
    public TypeResource getType() {
	return (TypeResource)type.evaluate();
    }
    
    public void setType(TypeResource type) {
	this.type.setValue(type);
    }
    
    public void giveActions(JMenu m) {
	super.giveActions(m);
	m.add(new JSeparator());
	m.add(
	new AbstractAction("List editor") {
	    public void actionPerformed(ActionEvent e) {
		new ListEditor(ListResource.this).show();
	    }
	}
	);
    }
    
    public abstract Resource elementAt(IntResource index);
    public abstract void setElement(IntResource index, Resource value);
    
    public synchronized Resource[] toArray() {
	Iterator iter = iterator();
	Resource[] ret = new Resource[ getCount().toInt() ];
	
	
	for (int i = 0; iter.hasNext(); i++ ) {
	    ret[i] = (Resource)iter.next();
	}
	
	return ret;
    }
    
    
    public abstract IntResource getCount();

    public Iterator iterator() {
	return new Iterator() {
	    private int current = 0;
	    
	    /**
	     * Returns <tt>true</tt> if the iteration has more elements. (In other
	     * words, returns <tt>true</tt> if <tt>next</tt> would return an element
	     * rather than throwing an exception.)
	     *
	     * @return <tt>true</tt> if the iterator has more elements.
	     */
	    public boolean hasNext() {
		if (getCount().toLong() > current)
		    return true;
		
		return false;
	    }
	    
	    /**
	     * Returns the next element in the iteration.
	     *
	     * @return the next element in the iteration.
	     * @exception NoSuchElementException iteration has no more elements.
	     */
	    public Object next() {
		Resource ret = elementAt(new IntPrimativeResource(current));
		current++;
		return ret;
	    }
	    
	    public void remove() throws UnsupportedOperationException {
		throw new UnsupportedOperationException();
	    }
	    
	    
	};
    }
    
    
}
