/**
 * JavaSmartPointer.java
 *
 Simple implementation of SmartPointer.
 Uses Java's class hierarchy to decide if a value is of the right type.
 
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.pointer;

import net.pengo.app.OpenFile;
import net.pengo.resource.QNodeResource;

public class JavaPointer extends SmartPointer {
    private String classname;
    private Class type;
    
    public JavaPointer(OpenFile openFile, String classname) {
	super(openFile);
	this.classname = classname;
	
	try {
	    type = Class.forName(classname);
	} catch (ClassNotFoundException e) {
	    //fixme: need proper handling
	    e.printStackTrace();
	}
    }
    
    public JavaPointer(OpenFile openFile, Class cls) {
	super(openFile);
	this.classname = cls.getName(); //fixme: is this the qualified name? should be
	this.type = cls;
    }
    
    
    public String toString() {
	if (getName()==null)
	    return "JavaPointer to " + getPrimarySource() + " = " + evalute() + " (" + classname + ")";
		
	return getName();
    }
    
    public Class getType() {
	return type;
    }
    
    public void setValue(QNodeResource value) {
	
	//fixme: need proper handling.
	//note: this replaces compile time checking with runtime checking. fixme: fix with assertions?
	if (!isValidValue(value)) {
	    System.out.println("error.. value is of wrong type. expected " + type + " but got " + value.getClass());
	}
	
	super.setValue(value);
    }
    
    public boolean isValidValue(QNodeResource value) {
	return type.isInstance(value.evalute());
    }
    
    public boolean isAssignableFrom(Class cl) {
	return (type.isAssignableFrom(cl));
    }
    
    //fixme: is name correct? is assignableTo really opposite to assignableFrom
    public boolean isAssignableTo(Class cl) {
	return (cl.isAssignableFrom(type));
    }
    
    
    
}

