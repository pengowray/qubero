/*
 * Created on Jan 1, 2004
 *
 * To change the template for this generated file go to
 * Window - Preferences - Java - Code Generation - Code and Comments
 */
package net.pengo.pointer;

import java.lang.reflect.Constructor;
import java.lang.reflect.InvocationTargetException;

import net.pengo.resource.Resource;
import net.pengo.resource.Resource;

/**
 * @author Peter Halasz
 */
public class ConstructorQFunction extends QFunction {
    private Constructor cons;
    
    /**
     * @param openFile
     */
    public ConstructorQFunction() {
        super();
    }
    
    
    public void setValue(Constructor cons) {
        this.cons = cons;
    }
    
    
    /* obj always = null */
    public void invoke(SmartPointer result, Resource obj, Resource[] param) {
        try {
            result.setValue((Resource)cons.newInstance(param));
        } catch (InstantiationException e) {
            // TODO: handle exception
            e.printStackTrace();
        } catch (InvocationTargetException e){
            e.printStackTrace();
        } catch (IllegalAccessException e) {
            e.printStackTrace();
        }
    }


    public Class[] callSignature() {
        return cons.getParameterTypes();
    }

    public Resource[] getSources() {
        // TODO Auto-generated method stub
        return null;
    }


    /* (non-Javadoc)
     * @see net.pengo.resource.QNodeResource#editProperties()
     */
    public void editProperties() {
        // TODO Auto-generated method stub
        
    }
}
