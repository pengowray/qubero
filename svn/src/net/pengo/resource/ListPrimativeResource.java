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
 * Created on Dec 30, 2003
 */
package net.pengo.resource;

import net.pengo.pointer.JavaPointer;
import net.pengo.pointer.MethodQFunction;
import net.pengo.pointer.QFunction;
import net.pengo.pointer.SmartPointer;

import net.pengo.resource.TypeResource;
import javax.swing.JSeparator;
import javax.swing.JMenu;
import javax.swing.AbstractAction;
import java.awt.event.ActionEvent;
import net.pengo.listEditor.ListEditor;
import java.util.Iterator;
/**
 * @author Peter Halasz
 *
 * perhaps better named ArrayResource. This should be used a lot more.
 *
 * Should have an abstract (list-ish) parent, that ListAddressedResource inherets from.
 * this class should be ArrayResource, i guess
 *
 */
public class ListPrimativeResource extends ListResource {
    
    public final SmartPointer count = new JavaPointer(net.pengo.resource.IntResource.class);

    public final MethodQFunction elementAt = new MethodQFunction();
    public final MethodQFunction setElement = new MethodQFunction();
    
    private Resource[] array;
    
    public ListPrimativeResource(TypeResource type) {
	super(type);
	
	this.count.setName("Count");
	this.count.addSink(this);
	this.count.setValue(new IntPrimativeResource(0));
	this.count.addAlertListenerer(new ResourceAlertListener() {
	    
	    public void valueChanged(Resource updated) {
		lengthChange();
	    }
	    
	    public void resourceRemoved(Resource removed) {
		//FIXME: should reset value to 0 ?
	    }
	    
	});
	
	elementAt.setName("element at");
	elementAt.addSink(this);
	
	setElement.setName("set element");
	setElement.addSink(this);
	
	try {
	    elementAt.setValue(getClass().getMethod("elementAt",new Class[]{IntResource.class}));
	    setElement.setValue(getClass().getMethod("setElement",new Class[]{IntResource.class, Resource.class}));
	} catch (NoSuchMethodException e) {
	    //FIXME: in theory, should be caught happen compile time
	    // TODO: handle exception
	    e.printStackTrace();
	}
	
    }
    
    public ListPrimativeResource(String classname){
	//FIXME: probably should be part of TypeRegistry
	this(StringToJavaType(classname));
    }
    
    private static TypeResource StringToJavaType(String classname) {
	
	Class cl = null;
	try {
	    cl = Class.forName(classname);
	    
	} catch (ClassNotFoundException e) {
	    //fixme: handle exception
	    e.printStackTrace();
	}
	
	return new JavaType(cl);
	
    }
    
    public Resource elementAt(IntResource index) {
	return array[index.getValue().intValue()];
    }
    
    public void setElement(IntResource index, Resource value) {
	array[index.getValue().intValue()] = value;
    }
    
    public Resource[] getSources() {
	return new Resource[] { count, type };
    }
    
    public QFunction[] getMethods(){
	return new QFunction[]{elementAt, setElement};
    }
    
    public void setValue(Resource[] array){
	//FIXME: this is serious mum.. just discard the old array without updating its resources sinks etc?
	this.array = array;
	this.count.setValue(new IntPrimativeResource(array.length));
	//fixme: array + count changes should be atomic
	alertChangeListenerer();
    }
    
    // triggered when length variable is changed
    private void lengthChange(){
	int len = ( (IntResource) count.evaluate() ).getValue().intValue();
	
	if (array == null) {
	    //FIXME: array length limited to int sizes
	    array = new Resource[len];
	} else {
	    //FIXME: shouldn't need this check -- lack of change should be handled by JavaPointer
	    if (array.length != len) {
		Resource[] newArray = new Resource[len];
		System.arraycopy(array,0,newArray,0,len<array.length?len:array.length);
		setValue(array);
	    }
	}
    }
    
    // check if this instance is null or void or not totally initialised or whatever
    //fixme: i don't like this much, but makes things easier for creating a ListResource with current GUI
    public boolean isVoid() {
	return true;
    }
    
    public void setCount(IntResource count) {
	this.count.setValue(count);
    }
    
    public IntResource getCount() {
	return (IntResource)count.evaluate();
    }
    
}
