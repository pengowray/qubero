/**
 * JavaSmartPointer.java
 *
 Simple implementation of SmartPointer.
 Uses Java's class hierarchy to decide if a value is of the right type.
 
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.pointer;

import net.pengo.resource.Resource;

public class JavaPointer extends SmartPointer {
    private String classname;
    private Class type;
    
    public JavaPointer(String classname) {
        this.classname = classname;
        
        try {
            type = Class.forName(classname);
        } catch (ClassNotFoundException e) {
            //fixme: need proper handling
            e.printStackTrace();
        }
    }
    
    public JavaPointer(Class cls) {
        this.classname = cls.getName(); //fixme: is this the qualified name? should be
        this.type = cls;
    }
    
    
    public String valueDesc() {
        
        String desc = evaluate().valueDesc();
        
        if (value.getName()==null)
            if (value.isPointer())
                return "(->"+desc+")";
            else
                return "("+desc+")";
        else
            if (value.isPointer())
                return value.getName() + " (->" + desc + ")";
            else
                return value.getName() + " (" + desc + ")";
    }
    
    public Class getType() {
        return type;
    }
    
    public void setValue(Resource value) {
        
        //fixme: need proper handling.
        //note: this replaces compile time checking with runtime checking. fixme: fix with assertions?
        if (!isValidValue(value)) {
            System.out.println("error.. value is of wrong type. expected " + type + " but got " + value.getClass());
        }
        
        super.setValue(value);
    }
    
    public boolean isValidValue(Resource value) {
        return type.isInstance(value.evaluate());
    }
    
    public boolean isAssignableFrom(Class cl) {
        return (type.isAssignableFrom(cl));
    }
    
    //fixme: is name correct? is assignableTo really opposite to assignableFrom
    public boolean isAssignableTo(Class cl) {
        return (cl.isAssignableFrom(type));
    }
    
    
    
}

