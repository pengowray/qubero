/*
 * Created on Jan 1, 2004
 *
 * To change the template for this generated file go to
 * Window - Preferences - Java - Code Generation - Code and Comments
 */
package net.pengo.pointer;

import java.lang.reflect.Method;

import net.pengo.dependency.QNode;
import net.pengo.resource.QNodeResource;

/**
 * @author Smiley
 *
 * To change the template for this generated type comment go to
 * Window - Preferences - Java - Code Generation - Code and Comments
 */
public class MethodQFunction extends QFunction {
    private Method method;
    
    /**
     * @param openFile
     */
    public MethodQFunction() {
        // TODO Auto-generated constructor stub
    }
    
    //wrap a method
    //eventually this should be replaced with some more formal way of converting types
    public void setValue(Method m) {
        this.method = m;
        //this.obj = obj;
        setName(m.getName()); //FIXME: umm? should i be doing this?
        //Class[] param = m.getParameterTypes();
        //Class ret = m.getReturnType();
    }

    public void invoke(SmartPointer result, QNodeResource obj, QNodeResource[] param){
        try {
            Object ret;
            ret = method.invoke(obj, param);
            result.setValue((QNodeResource)ret);
        } catch (Exception e) {
            //FIXME
            e.printStackTrace();
        }
    }

    /* (non-Javadoc)
     * @see net.pengo.pointer.QFunction#callSignature()
     */
    public Class[] callSignature() {
        return method.getParameterTypes();
    }

    public QNode[] getSources() {
        return null;
    }

    /* (non-Javadoc)
     * @see net.pengo.resource.QNodeResource#editProperties()
     */
    public void editProperties() {
        // TODO Auto-generated method stub
        
    }
    
    
}
